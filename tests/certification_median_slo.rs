// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! CPU-only same-cohort median integration; no GPU evidence is implied.
use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::time::{Duration, Instant};

#[test]
fn tool_evaluates_the_same_answered_cohort_at_its_median() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    );
    listener.set_nonblocking(true).unwrap();
    let mock = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        for delay in [10, 50] {
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::yield_now()
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                }
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let count = socket.read(&mut buffer).unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(delay));
            let body = r#"{"choices":[{"index":0,"message":{"content":"answer"},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"completion_tokens":8,"total_tokens":12}}"#;
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let prompt = dir.path().join("prompts.jsonl");
    let data = dir.path().join("data.jsonl");
    std::fs::write(&prompt, "{\"prompt\":\"hello\"}\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-llm"))
        .args([
            "--url",
            &url,
            "--api-key",
            "dummy",
            "--scenario",
            "median-test",
            "--num-requests",
            "2",
            "--concurrency",
            "1",
            "--mode",
            "chat",
            "--model",
            "fixture",
            "--max-tokens",
            "8",
            "--warmup-requests",
            "0",
            "--cert-e2e-median-slo",
            "--prompts",
        ])
        .arg(prompt)
        .arg("--data-log")
        .arg(&data)
        .arg("--debug-log")
        .arg(dir.path().join("debug.log"))
        .arg("--error-log")
        .arg(dir.path().join("error.log"))
        .output()
        .unwrap();
    mock.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let values: Vec<Value> = std::fs::read_to_string(data)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let rows: Vec<_> = values
        .iter()
        .filter(|r| {
            r["schema_version"]
                .as_str()
                .unwrap_or("")
                .contains(".request.")
                && r["phase"] == "measure"
        })
        .collect();
    let summary = values
        .iter()
        .find(|r| {
            r["schema_version"]
                .as_str()
                .unwrap_or("")
                .contains(".summary.")
        })
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r["answer_completed"] == true));
    let a = rows[0]["latency_s"].as_f64().unwrap();
    let b = rows[1]["latency_s"].as_f64().unwrap();
    assert_ne!(a, b);
    let median = a / 2.0 + b / 2.0;
    let actual = summary["goodput"]["thresholds_s"]["e2e"].as_f64().unwrap();
    assert!((actual - median).abs() < 1e-9);
    assert_eq!(summary["goodput"]["count"], 1);
    assert_eq!(summary["config"]["modality"]["cert_e2e_median_slo"], true);
}
