// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Observed concurrency must never exceed the configured cap (#189).
//!
//! Each modality binary runs many short requests against dummy-model-server
//! (strategic uses the in-repo mock server) and the test asserts that
//! `observed_concurrency.in_flight_max` and `in_flight_mean` are at most the
//! cap. Releasing the semaphore permit before the in-flight guard let the next
//! request enter first, so the gauge read cap+1. Skipped if `go` is missing.

mod common;

use common::{sine_wav, skip, spawn_dummy, summary_record};
use serde_json::Value;
use std::io::Write;
use std::path::Path;
use std::process::Command;

/// Enough short requests that a permit-before-guard release races reliably.
const REQUESTS: &str = "300";
/// Small server delay so requests overlap and the cap is engaged.
const DUMMY_LATENCY: &str = "-latency=2ms";

/// Assert the occupancy snapshot never exceeds `cap`.
fn assert_within_cap(observed: &Value, cap: u64, label: &str) {
    assert_eq!(
        observed["cap"].as_u64(),
        Some(cap),
        "{label}: cap {observed}"
    );
    let max = observed["in_flight_max"]
        .as_f64()
        .unwrap_or_else(|| panic!("{label}: in_flight_max missing: {observed}"));
    let mean = observed["in_flight_mean"]
        .as_f64()
        .unwrap_or_else(|| panic!("{label}: in_flight_mean missing: {observed}"));
    let cap = cap as f64;
    assert!(max <= cap, "{label}: in_flight_max {max} > cap {cap}");
    assert!(mean <= cap, "{label}: in_flight_mean {mean} > cap {cap}");
    assert!(max >= 1.0, "{label}: in_flight_max {max} < 1");
}

fn summary_observed(data_log: &Path, label: &str) -> Value {
    let summary = summary_record(data_log).unwrap_or_else(|| panic!("{label}: no summary"));
    summary["observed_concurrency"].clone()
}

fn common_args<'a>(url: &'a str, cap: &'a str, dir: &'a Path, scenario: &'a str) -> Vec<String> {
    let data_log = dir.join("out.jsonl");
    [
        "--url",
        url,
        "--api-key",
        "dummy",
        "--scenario",
        scenario,
        "--model",
        "dummy",
        "--num-requests",
        REQUESTS,
        "--concurrency",
        cap,
        "--data-log",
        data_log.to_str().unwrap(),
        "--debug-log",
        dir.join("debug.log").to_str().unwrap(),
        "--error-log",
        dir.join("error.log").to_str().unwrap(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn run_bin(bin: &str, args: &[String], label: &str) {
    let output = Command::new(bin).args(args).output().expect("run binary");
    assert!(
        output.status.success(),
        "{label} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_llm(url: &str, cap: u64) {
    let dir = tempfile::tempdir().expect("tmpdir");
    let prompts = dir.path().join("prompts.jsonl");
    std::fs::write(&prompts, "{\"prompt\":\"Hi\"}\n").expect("write prompts");
    let cap_s = cap.to_string();
    let mut args = common_args(url, &cap_s, dir.path(), "e2e-cap-llm");
    args.extend(
        [
            "--prompts",
            prompts.to_str().unwrap(),
            "--mode",
            "chat",
            "--max-tokens",
            "4",
            "--log-level",
            "error",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    let label = format!("llm cap {cap}");
    run_bin(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-llm"), &args, &label);
    let observed = summary_observed(&dir.path().join("out.jsonl"), &label);
    assert_within_cap(&observed, cap, &label);
}

#[test]
fn llm_observed_concurrency_never_exceeds_cap() {
    let Some(dummy) = spawn_dummy(&[DUMMY_LATENCY]) else {
        skip("go dummy-model-server not available");
        return;
    };
    for cap in [1, 4] {
        run_llm(&dummy.url("/v1"), cap);
    }
}

#[test]
fn vlm_observed_concurrency_never_exceeds_cap() {
    let Some(dummy) = spawn_dummy(&[DUMMY_LATENCY]) else {
        skip("go dummy-model-server not available");
        return;
    };
    for cap in [1u64, 4] {
        let dir = tempfile::tempdir().expect("tmpdir");
        let image = dir.path().join("pixel.png");
        std::fs::write(&image, common::tiny_png()).expect("write png");
        let prompts = dir.path().join("prompts.jsonl");
        let mut file = std::fs::File::create(&prompts).expect("create prompts");
        writeln!(
            file,
            r#"{{"prompt":"Describe this image","image_url":"{}"}}"#,
            image.to_str().unwrap()
        )
        .expect("write prompts");
        let url = dummy.url("/v1");
        let cap_s = cap.to_string();
        let mut args = common_args(&url, &cap_s, dir.path(), "e2e-cap-vlm");
        args.extend(
            [
                "--prompts",
                prompts.to_str().unwrap(),
                "--max-tokens",
                "4",
                "--log-level",
                "error",
            ]
            .iter()
            .map(|s| s.to_string()),
        );
        let label = format!("vlm cap {cap}");
        run_bin(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-vlm"), &args, &label);
        let observed = summary_observed(&dir.path().join("out.jsonl"), &label);
        assert_within_cap(&observed, cap, &label);
    }
}

#[test]
fn asr_observed_concurrency_never_exceeds_cap() {
    let Some(dummy) = spawn_dummy(&[DUMMY_LATENCY]) else {
        skip("go dummy-model-server not available");
        return;
    };
    for cap in [1u64, 4] {
        let dir = tempfile::tempdir().expect("tmpdir");
        let audio = dir.path().join("sample.wav");
        std::fs::write(&audio, sine_wav(0.5)).expect("write wav");
        let input = dir.path().join("input.jsonl");
        std::fs::write(
            &input,
            format!(
                "{}\n",
                serde_json::json!({
                    "id": "sample-1",
                    "path": audio.to_str().unwrap(),
                    "format": "wav",
                    "duration": 0.5
                })
            ),
        )
        .expect("write input");
        let url = dummy.url("/v1");
        let cap_s = cap.to_string();
        let mut args = common_args(&url, &cap_s, dir.path(), "e2e-cap-asr");
        args.extend(
            ["--input", input.to_str().unwrap(), "--log-level", "error"]
                .iter()
                .map(|s| s.to_string()),
        );
        let label = format!("asr cap {cap}");
        run_bin(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-asr"), &args, &label);
        let observed = summary_observed(&dir.path().join("out.jsonl"), &label);
        assert_within_cap(&observed, cap, &label);
    }
}

/// Imagegen already released the guard first; this pins that order.
#[test]
fn imagegen_observed_concurrency_never_exceeds_cap() {
    let Some(dummy) = spawn_dummy(&[DUMMY_LATENCY]) else {
        skip("go dummy-model-server not available");
        return;
    };
    for cap in [1u64, 4] {
        let dir = tempfile::tempdir().expect("tmpdir");
        let url = dummy.url("/v1");
        let cap_s = cap.to_string();
        let mut args = common_args(&url, &cap_s, dir.path(), "e2e-cap-imagegen");
        args.extend(
            [
                "--prompt",
                "a small test image",
                "--size",
                "64x64",
                "--artifact-dir",
                dir.path().join("artifacts").to_str().unwrap(),
                "--no-save-images",
            ]
            .iter()
            .map(|s| s.to_string()),
        );
        let label = format!("imagegen cap {cap}");
        run_bin(
            env!("CARGO_BIN_EXE_metrum-ai-bench-cli-imagegen"),
            &args,
            &label,
        );
        let observed = summary_observed(&dir.path().join("out.jsonl"), &label);
        assert_within_cap(&observed, cap, &label);
    }
}

#[test]
fn strategic_stage_observed_concurrency_never_exceeds_cap() {
    let server = common::spawn_mock(&["--latency-ms", "2"]);
    let address = server.address;

    let dir = tempfile::tempdir().expect("tmpdir");
    let html = dir.path().join("report.html");
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"))
        .args([
            "--url",
            &format!("http://{address}/v1/chat/completions"),
            "--model",
            "mock",
            "--requests-per-stage",
            REQUESTS,
            "--sweep",
            "1,4",
            "--sweep-by",
            "concurrency",
            "--html",
            html.to_str().unwrap(),
        ])
        .output()
        .expect("run strategic benchmark");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let summary: Value = serde_json::from_slice(&output.stdout).expect("summary JSON");
    let points = summary["points"].as_array().expect("points");
    assert_eq!(points.len(), 2);
    for (point, cap) in points.iter().zip([1u64, 4]) {
        assert_within_cap(
            &point["observed_concurrency"],
            cap,
            &format!("strategic stage {cap}"),
        );
    }
}
