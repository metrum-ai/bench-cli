// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Ordered, deterministic local SSE outcomes for reasoning-metric certification.
mod common;

use common::{request_records, summary_record};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::time::{Duration, Instant};

fn serve_request(mut socket: TcpStream, answered: bool, answer_only: bool, silent: bool) {
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut request = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let n = socket.read(&mut buffer).unwrap();
        assert!(n > 0, "request disconnected");
        request.extend_from_slice(&buffer[..n]);
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
    let mut frames = Vec::new();
    if !answer_only && !silent {
        frames.push(
            serde_json::json!({"choices":[{"index":0,"delta":{"reasoning_content":"think"}}]}),
        );
    }
    if answered && !silent {
        frames.push(serde_json::json!({"choices":[{"index":0,"delta":{"content":"answer"}}]}));
    }
    frames.push(serde_json::json!({"choices":[{"index":0,"finish_reason":if answered || silent { "stop" } else { "length" }}]}));
    frames.push(serde_json::json!({"choices":[],"usage":{"prompt_tokens":4,"completion_tokens":8,"total_tokens":12,"completion_tokens_details":{"reasoning_tokens":if answer_only { 0 } else { 7 }}}}));
    let mut body = String::new();
    for frame in frames {
        body.push_str(&format!("data: {frame}\n\n"));
    }
    body.push_str("data: [DONE]\n\n");
    write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
}

fn run_ordered(
    answered: &[bool],
    flags: &[&str],
    answer_only: bool,
    silent: bool,
) -> (bool, Vec<serde_json::Value>, serde_json::Value, String) {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    );
    let replies = answered.to_vec();
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        for answer in replies {
            let socket = loop {
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
            serve_request(socket, answer, answer_only, silent);
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let prompts = dir.path().join("prompts.jsonl");
    let data_log = dir.path().join("data.jsonl");
    std::fs::write(&prompts, "{\"prompt\":\"fixture\"}\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-llm"))
        .args([
            "--url",
            &url,
            "--api-key",
            "dummy",
            "--scenario",
            "catalog",
            "--num-requests",
            &answered.len().to_string(),
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
            "3",
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
        .arg(dir.path().join("error.log"))
        .args(flags)
        .output()
        .unwrap();
    server.join().unwrap();
    let records = request_records(&data_log);
    let summary = summary_record(&data_log).unwrap();
    (
        output.status.success(),
        records,
        summary,
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn assert_outcomes(
    records: &[serde_json::Value],
    summary: &serde_json::Value,
    answers: usize,
    strict: bool,
) {
    assert_eq!(records.len(), 16);
    assert_eq!(
        records
            .iter()
            .filter(|r| r["answer_completed"] == true)
            .count(),
        answers
    );
    assert_eq!(
        records
            .iter()
            .filter(|r| r["outcome"] == "reasoning_truncated")
            .count(),
        16 - answers
    );
    for row in records {
        let truncated = row["outcome"] == "reasoning_truncated";
        assert_eq!(row["error"].is_null(), !truncated || !strict);
        assert!(row["ttft_s"].as_f64().is_some());
        assert_eq!(row["ttft_source"], "stream");
        assert_eq!(row["ttft_visible_answer_s"].is_null(), truncated);
    }
    assert_eq!(summary["answered_requests"], answers);
    assert_eq!(
        summary["ttft_coverage"]["reasoning_truncated"],
        16 - answers
    );
    assert_eq!(summary["successes"], if strict { answers } else { 16 });
    assert_eq!(summary["goodput"]["count"], answers);
}

// BC-CLI-001: compatibility flags, source labels, audit-only relaxation and conflicts.
#[test]
fn bc_cli_001_ttft_flag_cross_product() {
    let flags: &[&[&str]] = &[
        &[],
        &["--infer-ttft-from-first-byte"],
        &["--ttft-fallback", "first-token"],
        &["--allow-missing-ttft"],
        &["--ttft-fallback", "first-token", "--allow-missing-ttft"],
    ];
    for option in flags {
        let (ok, rows, _, stderr) = run_ordered(&[true], option, false, false);
        assert!(ok, "{option:?}: {stderr}");
        let row = &rows[0];
        assert_eq!(row["ttft_source"], "stream");
        assert_eq!(
            row["ttft_s"].as_f64(),
            Some(
                row["first_reasoning_s"]
                    .as_f64()
                    .unwrap()
                    .min(row["ttft_visible_answer_s"].as_f64().unwrap())
            )
        );
        if option.contains(&"--ttft-fallback") {
            assert!(stderr.contains("deprecated"));
        }
    }
    let (default_ok, default_rows, _, _) = run_ordered(&[false], &[], false, true);
    assert!(!default_ok);
    assert!(default_rows[0]["ttft_s"].is_null());
    let (allow_ok, allowed, _, _) = run_ordered(&[false], &["--allow-missing-ttft"], false, true);
    assert!(allow_ok);
    assert_eq!(allowed[0]["error"]["kind"], "no_output_token");
    let (error_ok, _, _, _) = run_ordered(
        &[false],
        &["--allow-missing-ttft", "--fail-on-error"],
        false,
        true,
    );
    assert!(!error_ok);
    let (_, approx, _, _) = run_ordered(&[false], &["--infer-ttft-from-first-byte"], false, true);
    assert_eq!(approx[0]["ttft_source"], "first_byte_approx");
    assert_eq!(approx[0]["ttft_s"], approx[0]["first_byte_s"]);
    assert_eq!(approx[0]["error"]["kind"], "no_output_token");
    for binary in [
        env!("CARGO_BIN_EXE_metrum-ai-bench-cli-llm"),
        env!("CARGO_BIN_EXE_metrum-ai-bench-cli-vlm"),
        env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"),
    ] {
        let output = Command::new(binary)
            .args([
                "--url",
                "not-a-url",
                "--model",
                "stub",
                "--streaming",
                "--ttft-fallback",
                "first-token",
                "--infer-ttft-from-first-byte",
            ])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("cannot be used with"), "{stderr}");
    }
}

// BC-SSE-003: ordered mixed workloads and answer-only controls under all TTFT modes.
#[test]
fn bc_sse_003_mixed_order_and_answer_population() {
    for answers in [2usize, 16] {
        for reverse in [false, true] {
            let mut sequence: Vec<bool> = (0..16).map(|i| i < answers).collect();
            if reverse {
                sequence.reverse();
            }
            for flags in [
                &[][..],
                &["--ttft-fallback", "first-token"][..],
                &["--allow-missing-ttft"][..],
                &["--ttft-fallback", "first-token", "--allow-missing-ttft"][..],
                &["--infer-ttft-from-first-byte"][..],
            ] {
                let (ok, rows, summary, _) = run_ordered(&sequence, flags, answers == 16, false);
                assert!(ok, "{flags:?}");
                if answers == 16 {
                    assert!(rows.iter().all(|r| r["first_reasoning_s"].is_null()));
                }
                assert_outcomes(&rows, &summary, answers, false);
            }
        }
    }
}

// BC-CLI-002: all-truncated and mixed exits reflect the effective error policy.
#[test]
fn bc_cli_002_strict_exit_and_effective_error_matrix() {
    for answers in [0usize, 2] {
        let sequence: Vec<bool> = (0..16).map(|i| i < answers).collect();
        for (flags, strict) in [
            (&[][..], false),
            (&["--fail-on-error"][..], false),
            (&["--strict-reasoning-truncation"][..], true),
            (&["--ci-safety"][..], true),
        ] {
            let (ok, rows, summary, _) = run_ordered(&sequence, flags, false, false);
            assert_eq!(ok, !strict, "{flags:?}");
            assert_outcomes(&rows, &summary, answers, strict);
        }
    }
}
