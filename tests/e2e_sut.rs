// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! SUT block and hostname redaction end-to-end checks.

mod common;

use common::{skip, spawn_dummy, summary_record};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

fn llm_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_metrum-ai-bench-llm"))
}

fn write_prompts(dir: &std::path::Path) -> PathBuf {
    let prompts = dir.join("prompts.jsonl");
    let mut f = std::fs::File::create(&prompts).unwrap();
    writeln!(f, r#"{{"prompt":"Hi"}}"#).unwrap();
    prompts
}

fn base_args(url: &str, prompts: &str, data_log: &str, debug: &str, err: &str) -> Vec<String> {
    [
        "--url",
        url,
        "--api-key",
        "dummy",
        "--scenario",
        "sut-e2e",
        "--num-requests",
        "1",
        "--concurrency",
        "1",
        "--prompts",
        prompts,
        "--mode",
        "chat",
        "--streaming",
        "--model",
        "dummy",
        "--max-tokens",
        "8",
        "--data-log",
        data_log,
        "--debug-log",
        debug,
        "--error-log",
        err,
        "--log-level",
        "error",
        "--seed",
        "7",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

#[test]
fn sut_absent_is_null_with_notice() {
    let Some(dummy) = spawn_dummy(&[]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let url = dummy.url("/v1/chat/completions");
    let tmp = tempfile::tempdir().unwrap();
    let prompts = write_prompts(tmp.path());
    let data_log = tmp.path().join("out.jsonl");
    let output = Command::new(llm_bin())
        .args(base_args(
            &url,
            prompts.to_str().unwrap(),
            data_log.to_str().unwrap(),
            tmp.path().join("d.log").to_str().unwrap(),
            tmp.path().join("e.log").to_str().unwrap(),
        ))
        .output()
        .unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("sut: not provided"), "stderr={stderr}");
    let summary = summary_record(&data_log).expect("summary");
    assert!(summary["sut"].is_null(), "sut={}", summary["sut"]);
}

#[test]
fn sut_present_embedded_and_require_implies_redact() {
    let Some(dummy) = spawn_dummy(&[]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let url = dummy.url("/v1/chat/completions");
    let tmp = tempfile::tempdir().unwrap();
    let prompts = write_prompts(tmp.path());
    let sut = tmp.path().join("sut.json");
    std::fs::write(
        &sut,
        r#"{"name":"test-box","gpu":{"model":"L40S","count":1}}"#,
    )
    .unwrap();
    let data_log = tmp.path().join("out.jsonl");
    let mut args = base_args(
        &url,
        prompts.to_str().unwrap(),
        data_log.to_str().unwrap(),
        tmp.path().join("d.log").to_str().unwrap(),
        tmp.path().join("e.log").to_str().unwrap(),
    );
    args.extend([
        "--sut".into(),
        sut.to_str().unwrap().into(),
        "--require-sut".into(),
    ]);
    let status = Command::new(llm_bin()).args(&args).status().unwrap();
    assert!(status.success());
    let summary = summary_record(&data_log).expect("summary");
    assert_eq!(summary["sut"]["name"], "test-box");
    assert_eq!(summary["sut"]["provenance"], "declared");
    assert!(summary["environment"]["hostname"].is_null());
}

#[test]
fn require_sut_without_file_sends_no_requests() {
    let Some(dummy) = spawn_dummy(&[]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let url = dummy.url("/v1/chat/completions");
    let tmp = tempfile::tempdir().unwrap();
    let prompts = write_prompts(tmp.path());
    let data_log = tmp.path().join("out.jsonl");
    let mut args = base_args(
        &url,
        prompts.to_str().unwrap(),
        data_log.to_str().unwrap(),
        tmp.path().join("d.log").to_str().unwrap(),
        tmp.path().join("e.log").to_str().unwrap(),
    );
    args.push("--require-sut".into());
    let status = Command::new(llm_bin()).args(&args).status().unwrap();
    assert!(!status.success());
    assert!(
        !data_log.exists() || std::fs::metadata(&data_log).map(|m| m.len()).unwrap_or(0) == 0,
        "no results should be written when --require-sut fails closed"
    );
}

#[test]
fn redact_hostname_alone() {
    let Some(dummy) = spawn_dummy(&[]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let url = dummy.url("/v1/chat/completions");
    let tmp = tempfile::tempdir().unwrap();
    let prompts = write_prompts(tmp.path());
    let data_log = tmp.path().join("out.jsonl");
    let mut args = base_args(
        &url,
        prompts.to_str().unwrap(),
        data_log.to_str().unwrap(),
        tmp.path().join("d.log").to_str().unwrap(),
        tmp.path().join("e.log").to_str().unwrap(),
    );
    args.push("--redact-hostname".into());
    let status = Command::new(llm_bin()).args(&args).status().unwrap();
    assert!(status.success());
    let summary = summary_record(&data_log).expect("summary");
    assert!(summary["environment"]["hostname"].is_null());
    assert!(summary["sut"].is_null());
}
