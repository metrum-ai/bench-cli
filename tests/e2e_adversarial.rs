// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Adversarial SSE framing and Ctrl-C behavior against dummy-model-server.
//! Skipped if `go` is missing.

mod common;

use common::{request_records, skip, spawn_dummy, summary_record};
use serde_json::Value;
use std::io::Write;
use std::process::Command;
use std::time::{Duration, Instant};

fn llm_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-llm"))
}

struct Fixture {
    _dir: tempfile::TempDir,
    prompts: std::path::PathBuf,
    data_log: std::path::PathBuf,
    debug_log: std::path::PathBuf,
    error_log: std::path::PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("tmpdir");
    let prompts = dir.path().join("prompts.jsonl");
    let mut file = std::fs::File::create(&prompts).expect("create prompts");
    writeln!(file, r#"{{"prompt":"Hi"}}"#).expect("write prompt");
    Fixture {
        prompts,
        data_log: dir.path().join("out.jsonl"),
        debug_log: dir.path().join("debug.log"),
        error_log: dir.path().join("error.log"),
        _dir: dir,
    }
}

fn llm_args(fixture: &Fixture, url: &str, requests: u32, extra: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = [
        "--url",
        url,
        "--api-key",
        "dummy",
        "--scenario",
        "adversarial",
        "--num-requests",
        &requests.to_string(),
        "--concurrency",
        "1",
        "--prompts",
        fixture.prompts.to_str().unwrap(),
        "--mode",
        "chat",
        "--streaming",
        "--model",
        "dummy",
        "--max-tokens",
        "10",
        "--data-log",
        fixture.data_log.to_str().unwrap(),
        "--debug-log",
        fixture.debug_log.to_str().unwrap(),
        "--error-log",
        fixture.error_log.to_str().unwrap(),
        "--log-level",
        "error",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.extend(extra.iter().map(|s| s.to_string()));
    args
}

fn run_llm(fixture: &Fixture, url: &str, requests: u32, extra: &[&str]) -> bool {
    Command::new(llm_bin())
        .args(llm_args(fixture, url, requests, extra))
        .status()
        .expect("run llm")
        .success()
}

/// SSE frames flushed mid-event must be reassembled, not dropped or
/// double-counted.
#[test]
fn split_sse_frames_are_reassembled() {
    let Some(dummy) = spawn_dummy(&["-split-sse", "-latency", "60ms", "-chunk-interval", "10ms"])
    else {
        skip("go dummy-model-server not available");
        return;
    };
    let fixture = fixture();
    assert!(run_llm(
        &fixture,
        &dummy.url("/v1/chat/completions"),
        2,
        &[]
    ));

    let records = request_records(&fixture.data_log);
    assert_eq!(records.len(), 2);
    for record in &records {
        assert!(record["error"].is_null(), "split frames produced an error");
        let ttft = record["ttft_s"].as_f64().expect("ttft_s");
        assert!(
            (0.040..0.500).contains(&ttft),
            "ttft {ttft}s outside dummy bounds with split frames"
        );
        let itl = record["itl_s"].as_array().expect("itl_s");
        // 10 max tokens => at most 9 intervals; a parser that emitted a token
        // per flushed fragment would report more.
        assert!(
            !itl.is_empty() && itl.len() <= 9,
            "{} ITL samples for 10 tokens",
            itl.len()
        );
    }
}

/// A stream that never sends `data: [DONE]` still terminates and is measured.
#[test]
fn missing_done_sentinel_still_completes() {
    let Some(dummy) = spawn_dummy(&["-omit-done", "-chunk-interval", "5ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let fixture = fixture();
    let start = Instant::now();
    assert!(run_llm(
        &fixture,
        &dummy.url("/v1/chat/completions"),
        2,
        &[]
    ));
    assert!(
        start.elapsed() < Duration::from_secs(30),
        "run hung waiting for a [DONE] that never arrives"
    );

    let records = request_records(&fixture.data_log);
    assert_eq!(records.len(), 2);
    for record in &records {
        assert!(record["latency_s"].as_f64().expect("latency_s") > 0.0);
    }
}

/// A stream carrying only a role delta has no visible output, which is an
/// error rather than a zero-latency success.
#[test]
fn role_only_stream_is_reported_as_no_output_token() {
    let Some(dummy) = spawn_dummy(&["-role-only"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let fixture = fixture();
    // The binary may exit non-zero when every request fails; the records are
    // what this test is about.
    let _ = run_llm(&fixture, &dummy.url("/v1/chat/completions"), 2, &[]);

    let records = request_records(&fixture.data_log);
    assert!(!records.is_empty(), "no records for role-only stream");
    for record in &records {
        assert!(
            record["ttft_s"].is_null(),
            "role delta must not count as first token"
        );
        let error = &record["error"];
        assert!(!error.is_null(), "role-only stream recorded as success");
        let kind = error_kind(error);
        assert_eq!(
            kind, "no_output_token",
            "expected no_output_token, got {error}"
        );
    }
}

/// Reasoning deltas are timed separately and must not be mistaken for the
/// first visible token.
#[test]
fn reasoning_deltas_are_separated_from_ttft() {
    let Some(dummy) = spawn_dummy(&["-reasoning", "-latency", "40ms", "-chunk-interval", "10ms"])
    else {
        skip("go dummy-model-server not available");
        return;
    };
    let fixture = fixture();
    assert!(run_llm(
        &fixture,
        &dummy.url("/v1/chat/completions"),
        2,
        &[]
    ));

    let records = request_records(&fixture.data_log);
    assert_eq!(records.len(), 2);
    for record in &records {
        let first_reasoning = record["first_reasoning_s"]
            .as_f64()
            .expect("first_reasoning_s");
        let ttft = record["ttft_s"].as_f64().expect("ttft_s");
        assert!(
            first_reasoning <= ttft,
            "reasoning at {first_reasoning}s reported after first token at {ttft}s"
        );
        assert!(
            ttft > first_reasoning,
            "ttft {ttft}s equals the reasoning timestamp, so reasoning was counted as output"
        );
    }
}

/// Ctrl-C stops issuing, keeps the records already written, and marks the
/// summary partial instead of reporting a truncated run as complete.
#[test]
fn ctrl_c_writes_a_partial_summary() {
    let Some(dummy) = spawn_dummy(&["-latency", "250ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let fixture = fixture();
    let requests = 40;
    let mut child = Command::new(llm_bin())
        .args(llm_args(
            &fixture,
            &dummy.url("/v1/chat/completions"),
            requests,
            &[],
        ))
        .spawn()
        .expect("spawn llm");

    // Let a few requests land, then interrupt.
    std::thread::sleep(Duration::from_millis(1200));
    let signalled = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .expect("send SIGINT")
        .success();
    assert!(signalled, "could not deliver SIGINT");

    let start = Instant::now();
    let status = loop {
        match child.try_wait().expect("wait") {
            Some(status) => break status,
            None if start.elapsed() > Duration::from_secs(60) => {
                let _ = child.kill();
                panic!("binary did not exit after SIGINT");
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    assert!(status.success(), "interrupted run exited with {status}");

    let records = request_records(&fixture.data_log);
    assert!(
        !records.is_empty(),
        "records were not flushed incrementally"
    );
    assert!(
        records.len() < requests as usize,
        "{} of {requests} requests completed; the interrupt did nothing",
        records.len()
    );

    let summary = summary_record(&fixture.data_log).expect("partial summary");
    assert_eq!(
        summary["partial"],
        Value::Bool(true),
        "interrupted run was not marked partial"
    );
    assert_eq!(
        summary["latency_s"]["n"].as_u64().expect("n") as usize,
        records
            .iter()
            .filter(|r| r["error"].is_null() && r["phase"] != "warmup")
            .count(),
        "partial summary disagrees with the records on disk"
    );
}

/// `RequestError` serializes with an internal `kind` tag.
fn error_kind(error: &Value) -> String {
    error["kind"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| error.to_string())
}

#[test]
fn llm_mid_stream_error_is_api_error() {
    let Some(dummy) = spawn_dummy(&["-error-rate", "1"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let fixture = fixture();
    assert!(run_llm(
        &fixture,
        &dummy.url("/v1/chat/completions"),
        1,
        &[]
    ));
    let records = request_records(&fixture.data_log);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["error"]["kind"], "api_error");
}
