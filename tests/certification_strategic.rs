// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Deterministic strategic stage and export certification.
use metrum_ai_bench::strategic::BenchRecord;
use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::time::{Duration, Instant};

fn run_sweep(export_failure: bool) -> (std::process::Output, Vec<BenchRecord>) {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    );
    let mock = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        for index in 0..4 {
            let mut socket = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::yield_now()
                    }
                    Err(e) => panic!("mock accept: {e}"),
                }
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut buf = [0; 4096];
            loop {
                let n = socket.read(&mut buf).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buf[..n]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let size: usize = headers
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    if request.len() >= end + 4 + size {
                        break;
                    }
                }
            }
            let (status, body) = if index < 2 {
                (
                    "200 OK",
                    r#"{"choices":[{"index":0,"message":{"content":"answer"},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"completion_tokens":8,"total_tokens":12}}"#,
                )
            } else {
                (
                    "503 Service Unavailable",
                    r#"{"error":{"message":"fixture error"}}"#,
                )
            };
            write!(socket, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let csv = if export_failure {
        dir.path().to_owned()
    } else {
        dir.path().join("requests.csv")
    };
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"))
        .args([
            "--url",
            &url,
            "--api-key",
            "dummy",
            "--model",
            "stub",
            "--max-tokens",
            "8",
            "--sweep",
            "1,2",
            "--requests-per-stage",
            "2",
            "--slo",
            "e2e=5s",
            "--fail-on-error",
        ])
        .arg("--csv")
        .arg(&csv)
        .arg("--html")
        .arg(dir.path().join("report.html"))
        .output()
        .unwrap();
    mock.join().unwrap();
    let rows = if export_failure {
        Vec::new()
    } else {
        csv::Reader::from_path(csv)
            .unwrap()
            .deserialize()
            .map(Result::unwrap)
            .collect()
    };
    (output, rows)
}

#[test]
fn strategic_input_only_usage_remains_available() {
    for (kind, body) in [
        (
            "embeddings",
            r#"{"data":[{"embedding":[0.1],"index":0}],"usage":{"prompt_tokens":4,"total_tokens":4}}"#,
        ),
        (
            "rerank",
            r#"{"results":[{"index":0,"relevance_score":0.5}],"usage":{"total_tokens":4}}"#,
        ),
    ] {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/fixture", listener.local_addr().unwrap());
        let mock = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(15);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::yield_now()
                    }
                    Err(e) => panic!("mock accept: {e}"),
                }
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut data = Vec::new();
            let mut buf = [0; 4096];
            loop {
                let n = socket.read(&mut buf).unwrap();
                assert!(n > 0);
                data.extend_from_slice(&buf[..n]);
                if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&data[..end]);
                    let size: usize = headers
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    if data.len() >= end + 4 + size {
                        break;
                    }
                }
            }
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        });
        let dir = tempfile::tempdir().unwrap();
        let csv = dir.path().join("requests.csv");
        let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"))
            .args([
                "--url",
                &url,
                "--api-key",
                "dummy",
                "--model",
                "stub",
                "--kind",
                kind,
                "--sweep",
                "1",
                "--requests-per-stage",
                "1",
            ])
            .arg("--csv")
            .arg(&csv)
            .arg("--html")
            .arg(dir.path().join("report.html"))
            .output()
            .unwrap();
        mock.join().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let summary: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(summary["points"][0]["prompt_tokens_total"], 4);
        assert!(summary["points"][0]["input_tokens_per_second"].is_number());
        assert!(summary["points"][0]["completion_tokens_per_second"].is_null());
        let records: Vec<BenchRecord> = csv::Reader::from_path(csv)
            .unwrap()
            .deserialize()
            .map(Result::unwrap)
            .collect();
        assert_eq!(records.len(), 1);
        assert!(!records[0].usage_missing);
        assert_eq!(records[0].input_tokens, 4);
    }
}

// BC-MODE-001: complete staged output is emitted before exports, even when
// export subsequently fails; the successful export retains failed rows.
#[test]
fn bc_mode_001_strategic_sweep_and_exports() {
    let (output, rows) = run_sweep(false);
    assert!(!output.status.success());
    let summary: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        summary["schema_version"],
        "metrum-ai-bench-cli.strategic.v3"
    );
    let points = summary["points"].as_array().unwrap();
    assert_eq!(points.len(), 2);
    assert_eq!(
        (points[0]["n"].as_u64(), points[0]["errors"].as_u64()),
        (Some(2), Some(0))
    );
    assert_eq!(
        (points[1]["n"].as_u64(), points[1]["errors"].as_u64()),
        (Some(2), Some(2))
    );
    assert_eq!(points[0]["goodput"], points[0]["throughput"]);
    assert_eq!(points[1]["goodput"], 0.0);
    assert_eq!(rows.len(), 4);
    assert_eq!(
        rows.iter().filter(|r| r.stage == 1.0 && r.success).count(),
        2
    );
    assert_eq!(
        rows.iter()
            .filter(|r| r.stage == 2.0 && !r.success && r.error.is_some())
            .count(),
        2
    );
    let (failed_export, _) = run_sweep(true);
    assert!(!failed_export.status.success());
    let before_export: Value = serde_json::from_slice(&failed_export.stdout).unwrap();
    assert_eq!(before_export["points"].as_array().unwrap().len(), 2);
}
