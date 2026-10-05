// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! #192 end-to-end checks for server-reported reasoning tokens against
//! dummy-model-server in `-strict-media` mode (Metrum AI Bench). Skipped if
//! `go` is missing.

mod common;

use common::{request_records, skip, spawn_dummy, summary_record};
use serde_json::Value;
use std::path::Path;
use std::process::Command;

const MAX_TOKENS: u64 = 8;
const REASONING: u64 = 5;

fn run_llm(url: &str, dir: &Path, streaming: bool) -> std::path::PathBuf {
    let prompts = dir.join("prompts.jsonl");
    std::fs::write(&prompts, "{\"prompt\":\"Hi\"}\n").expect("prompts");
    let data_log = dir.join(if streaming {
        "stream.jsonl"
    } else {
        "unary.jsonl"
    });
    let max_tokens = MAX_TOKENS.to_string();
    let mut command = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-llm"));
    command.args([
        "--url",
        url,
        "--api-key",
        "dummy",
        "--scenario",
        "reasoning-tokens",
        "--num-requests",
        "2",
        "--concurrency",
        "1",
        "--prompts",
        prompts.to_str().expect("utf8"),
        "--mode",
        "chat",
        "--model",
        "dummy",
        "--max-tokens",
        &max_tokens,
        "--data-log",
        data_log.to_str().expect("utf8"),
        "--debug-log",
        dir.join("debug.log").to_str().expect("utf8"),
        "--error-log",
        dir.join("error.log").to_str().expect("utf8"),
        "--log-level",
        "error",
    ]);
    if streaming {
        command.arg("--streaming");
    }
    let status = command.status().expect("run llm");
    assert!(status.success(), "llm bench failed");
    data_log
}

fn assert_reported(data_log: &Path, label: &str) {
    let records = request_records(data_log);
    assert_eq!(records.len(), 2, "{label}");
    for record in &records {
        // Values match the dummy usage payload exactly.
        assert_eq!(record["reasoning_tokens"], REASONING, "{label}");
        assert_eq!(
            record["completion_tokens"],
            MAX_TOKENS + REASONING,
            "{label}"
        );
        assert_eq!(record["visible_completion_tokens"], MAX_TOKENS, "{label}");
    }
    let summary = summary_record(data_log).expect("summary");
    assert_eq!(summary["reasoning_tokens"]["n"], 2, "{label}");
    assert_eq!(
        summary["reasoning_tokens"]["avg"], REASONING as f64,
        "{label}"
    );
    assert_eq!(summary["reasoning_tokens_total"], 2 * REASONING, "{label}");
    assert_eq!(summary["visible_completion_tokens"]["n"], 2, "{label}");
    assert_eq!(
        summary["visible_completion_tokens_total"],
        2 * MAX_TOKENS,
        "{label}"
    );
}

#[test]
fn llm_reasoning_tokens_match_dummy_usage() {
    let Some(dummy) = spawn_dummy(&[
        "-reasoning-tokens",
        &REASONING.to_string(),
        "-latency",
        "10ms",
        "-chunk-interval",
        "2ms",
    ]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let dir = tempfile::tempdir().expect("tmpdir");
    let url = dummy.url("/v1/chat/completions");
    assert_reported(&run_llm(&url, dir.path(), true), "streaming");
    assert_reported(&run_llm(&url, dir.path(), false), "non-streaming");
}

#[test]
fn llm_absent_reasoning_tokens_are_null_not_zero() {
    // `-reasoning` streams reasoning text but reports no usage details.
    let Some(dummy) = spawn_dummy(&["-reasoning", "-latency", "10ms", "-chunk-interval", "2ms"])
    else {
        skip("go dummy-model-server not available");
        return;
    };
    let dir = tempfile::tempdir().expect("tmpdir");
    let data_log = run_llm(&dummy.url("/v1/chat/completions"), dir.path(), true);
    for record in request_records(&data_log) {
        let object = record.as_object().expect("record object");
        assert!(object.contains_key("reasoning_tokens"));
        assert!(record["reasoning_tokens"].is_null(), "{record}");
        assert!(record["visible_completion_tokens"].is_null(), "{record}");
        assert_eq!(record["completion_tokens"], MAX_TOKENS);
    }
    let summary = summary_record(&data_log).expect("summary");
    assert_eq!(summary["reasoning_tokens"]["n"], 0);
    assert!(summary["reasoning_tokens_total"].is_null());
    assert!(summary["visible_completion_tokens_total"].is_null());
}

#[test]
fn strategic_points_report_reasoning_tokens() {
    let Some(dummy) = spawn_dummy(&[
        "-reasoning-tokens",
        &REASONING.to_string(),
        "-latency",
        "10ms",
        "-chunk-interval",
        "2ms",
    ]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let dir = tempfile::tempdir().expect("tmpdir");
    let prompts = dir.path().join("prompts.jsonl");
    std::fs::write(&prompts, "{\"prompt\":\"alpha\"}\n").expect("prompts");
    let csv = dir.path().join("records.csv");
    for streaming in [true, false] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"));
        command
            .args([
                "--url",
                &dummy.url("/v1/chat/completions"),
                "--model",
                "dummy",
                "--prompts",
                prompts.to_str().expect("utf8"),
                "--max-tokens",
                &MAX_TOKENS.to_string(),
                "--sweep",
                "2",
                "--requests-per-stage",
                "3",
            ])
            .arg("--csv")
            .arg(&csv)
            .arg("--html")
            .arg(dir.path().join("report.html"));
        if streaming {
            command.arg("--streaming");
        }
        let output = command.output().expect("run strategic");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let summary: Value = serde_json::from_slice(&output.stdout).expect("summary");
        let point = &summary["points"][0];
        assert_eq!(point["reasoning_tokens"]["n"], 3, "streaming={streaming}");
        assert_eq!(point["reasoning_tokens"]["avg"], REASONING as f64);
        assert_eq!(point["reasoning_tokens_total"], 3 * REASONING);
        assert_eq!(point["visible_completion_tokens"]["avg"], MAX_TOKENS as f64);
        let rows = std::fs::read_to_string(&csv).expect("csv");
        let header = rows.lines().next().expect("header");
        assert!(header.ends_with(",reasoning_tokens"), "{header}");
        assert!(rows
            .lines()
            .skip(1)
            .all(|line| line.ends_with(&format!(",{REASONING}"))));
    }
}
