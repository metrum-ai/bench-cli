// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Scripted HTTP, SSE and timeout failures around a valid completion.
mod common;
use common::{request_records, summary_record};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
enum Failure {
    Http,
    Malformed,
    PartialUsage,
    Disconnect,
    Timeout,
}

fn run_case(failure: Failure, before: bool, flag: Option<&str>) {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    );
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(15);
        for index in 0..2 {
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
            let fail = (index == 0) == before;
            if fail && matches!(failure, Failure::Timeout) {
                let usage = "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":3,\"total_tokens\":7,\"completion_tokens_details\":{\"reasoning_tokens\":2}}}\n\n";
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n{usage}").unwrap();
                socket.flush().unwrap();
                std::thread::sleep(Duration::from_millis(1500));
                continue;
            }
            let (status, body) = if !fail {
                ("200 OK", "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"answer\"}}]}\n\ndata: {\"choices\":[{\"index\":0,\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":2,\"total_tokens\":6,\"completion_tokens_details\":{\"reasoning_tokens\":0}}}\n\ndata: [DONE]\n\n")
            } else {
                match failure {
                Failure::Http => ("503 Service Unavailable", "{}"),
                Failure::Malformed => ("200 OK", "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":3,\"total_tokens\":7,\"completion_tokens_details\":{\"reasoning_tokens\":2}}}\n\ndata: {broken\n\ndata: [DONE]\n\n"),
                Failure::PartialUsage => ("200 OK", "data: {\"choices\":[],\"usage\":{\"completion_tokens\":3,\"completion_tokens_details\":{\"reasoning_tokens\":2}}}\n\ndata: {broken\n\n"),
                Failure::Disconnect => ("200 OK", "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"partial\"}}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":3,\"total_tokens\":7,\"completion_tokens_details\":{\"reasoning_tokens\":2}}}\n\n"),
                Failure::Timeout => unreachable!(),
            }
            };
            write!(socket, "HTTP/1.1 {status}\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let prompts = dir.path().join("prompts.jsonl");
    let data_log = dir.path().join("data.jsonl");
    std::fs::write(&prompts, "{\"prompt\":\"fixture\"}\n").unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-llm"));
    cmd.args([
        "--url",
        &url,
        "--api-key",
        "dummy",
        "--scenario",
        "catalog",
        "--num-requests",
        "2",
        "--concurrency",
        "1",
        "--mode",
        "chat",
        "--streaming",
        "--model",
        "stub",
        "--max-tokens",
        "8",
        "--request-timeout",
        "1",
        "--log-level",
        "error",
    ])
    .arg("--prompts")
    .arg(&prompts)
    .arg("--data-log")
    .arg(&data_log)
    .arg("--debug-log")
    .arg(dir.path().join("debug.log"))
    .arg("--error-log")
    .arg(dir.path().join("error.log"));
    if let Some(flag) = flag {
        cmd.arg(flag);
    }
    let started = Instant::now();
    let output = cmd.output().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{failure:?} hung"
    );
    server.join().unwrap();
    assert_eq!(
        output.status.success(),
        flag.is_none(),
        "{failure:?} {flag:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows = request_records(&data_log);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows.iter().filter(|r| r["error"].is_null()).count(), 1);
    let failed = rows.iter().find(|r| !r["error"].is_null()).unwrap();
    let kind = match failure {
        Failure::Http => "http_status",
        Failure::Malformed | Failure::PartialUsage => "parse_error",
        Failure::Disconnect => "stream_truncated",
        Failure::Timeout => "timeout",
    };
    assert_eq!(failed["error"]["kind"], kind);
    assert_ne!(failed["answer_completed"], true);
    if matches!(
        failure,
        Failure::Malformed | Failure::Disconnect | Failure::Timeout
    ) {
        assert_eq!(failed["usage_missing"], false);
        assert_eq!(failed["prompt_tokens"], 4);
        assert_eq!(failed["completion_tokens"], 3);
        assert_eq!(failed["reasoning_tokens"], 2);
        assert_eq!(failed["visible_completion_tokens"], 1);
        assert_eq!(failed["usage_prompt_tokens_present"], true);
        assert_eq!(failed["usage_completion_tokens_present"], true);
    } else {
        assert_eq!(failed["usage_missing"], true);
        if matches!(failure, Failure::PartialUsage) {
            assert_eq!(failed["usage_prompt_tokens_present"], false);
            assert_eq!(failed["usage_completion_tokens_present"], true);
            assert_eq!(failed["completion_tokens"], 3);
        }
    }
    let summary = summary_record(&data_log).unwrap();
    assert_eq!(summary["attempted"], 2);
    assert_eq!(summary["successes"], 1);
    assert_eq!(summary["errors"], 1);
    assert_eq!(summary["errors_by_type"][kind], 1);
    if matches!(
        failure,
        Failure::Malformed | Failure::Disconnect | Failure::Timeout
    ) {
        assert_eq!(summary["completion_tokens_total"], 5);
        assert!(summary["completion_tokens_per_second"].is_number());
        let window = summary["window_seconds"].as_f64().unwrap();
        assert!(
            (summary["non_reasoning_completion_tokens_per_second"]
                .as_f64()
                .unwrap()
                - 3.0 / window)
                .abs()
                < 1e-9
        );
    } else {
        assert!(summary["completion_tokens_per_second"].is_null());
    }
}

/// Exercise usage-before-error propagation through the VLM and strategic paths.
#[test]
fn failed_stream_usage_survives_all_modality_exports() {
    for (strategic, partial) in [(false, false), (false, true), (true, false), (true, true)] {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let url = format!(
            "http://{}/v1/chat/completions",
            listener.local_addr().unwrap()
        );
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
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
            let body = if partial {
                "data: {\"choices\":[],\"usage\":{\"completion_tokens\":3,\"completion_tokens_details\":{\"reasoning_tokens\":2}}}\n\ndata: {broken\n\n"
            } else {
                "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":3,\"total_tokens\":7,\"completion_tokens_details\":{\"reasoning_tokens\":2}}}\n\ndata: {broken\n\n"
            };
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        });
        let dir = tempfile::tempdir().unwrap();
        let csv = dir.path().join("rows.csv");
        let data_log = dir.path().join("data.jsonl");
        let mut cmd = if strategic {
            let mut c = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"));
            c.args(["--sweep", "1", "--requests-per-stage", "1"])
                .arg("--csv")
                .arg(&csv)
                .arg("--html")
                .arg(dir.path().join("report.html"));
            c
        } else {
            let image = dir.path().join("pixel.png");
            std::fs::write(&image, common::tiny_png()).unwrap();
            let prompts = dir.path().join("prompts.jsonl");
            std::fs::write(
                &prompts,
                format!(
                    "{{\"prompt\":\"fixture\",\"image_url\":{}}}\n",
                    serde_json::to_string(image.to_str().unwrap()).unwrap()
                ),
            )
            .unwrap();
            let mut c = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-vlm"));
            c.args([
                "--scenario",
                "catalog",
                "--num-requests",
                "1",
                "--concurrency",
                "1",
                "--log-level",
                "error",
            ])
            .arg("--prompts")
            .arg(&prompts)
            .arg("--data-log")
            .arg(&data_log)
            .arg("--debug-log")
            .arg(dir.path().join("debug.log"))
            .arg("--error-log")
            .arg(dir.path().join("error.log"));
            c
        };
        let output = cmd
            .args([
                "--url",
                &url,
                "--api-key",
                "dummy",
                "--model",
                "stub",
                "--max-tokens",
                "8",
                "--streaming",
                "--allow-missing-ttft",
                "--fail-on-error",
            ])
            .output()
            .unwrap();
        server.join().unwrap();
        assert!(!output.status.success());
        if strategic {
            let rows: Vec<metrum_ai_bench::strategic::BenchRecord> = csv::Reader::from_path(csv)
                .unwrap()
                .deserialize()
                .map(Result::unwrap)
                .collect();
            assert_eq!(rows.len(), 1);
            assert!(!rows[0].success);
            assert_eq!(rows[0].usage_missing, partial);
            assert_eq!(rows[0].usage_prompt_tokens_present, Some(!partial));
            assert_eq!(rows[0].usage_completion_tokens_present, Some(true));
            assert_eq!(
                (
                    rows[0].input_tokens,
                    rows[0].output_tokens,
                    rows[0].reasoning_tokens
                ),
                (if partial { 0 } else { 4 }, 3, Some(2))
            );
        } else {
            let rows = request_records(&data_log);
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0]["error"]["kind"], "parse_error");
            assert_eq!(rows[0]["usage_missing"], partial);
            assert_eq!(rows[0]["usage_prompt_tokens_present"], !partial);
            assert_eq!(rows[0]["usage_completion_tokens_present"], true);
            assert_eq!(rows[0]["completion_tokens"], 3);
            assert_eq!(rows[0]["reasoning_tokens"], 2);
            assert_eq!(rows[0]["visible_completion_tokens"], 1);
        }
    }
}

// BC-SSE-005: each failure occurs before and after a success under all gates.
#[test]
fn bc_sse_005_transport_framing_and_timeout_failures() {
    for failure in [
        Failure::Http,
        Failure::Malformed,
        Failure::PartialUsage,
        Failure::Disconnect,
        Failure::Timeout,
    ] {
        for before in [false, true] {
            for flag in [None, Some("--fail-on-error"), Some("--ci-safety")] {
                run_case(failure, before, flag);
            }
        }
    }
}
