// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Image generation end-to-end checks against dummy-model-server.
//! Skipped if `go` is missing.

mod common;

use common::{request_records, skip, spawn_dummy, summary_record};
use serde_json::Value;
use std::process::Command;

fn shared_request_records(data_log: &std::path::Path) -> Vec<Value> {
    request_records(data_log)
        .into_iter()
        .filter(|v| {
            v.get("schema_version")
                .and_then(Value::as_str)
                .is_some_and(|s| s.starts_with("metrum-ai-bench-cli.request."))
        })
        .collect()
}

fn imagegen_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-imagegen"))
}

struct Fixture {
    _dir: tempfile::TempDir,
    data_log: std::path::PathBuf,
    summary_json: std::path::PathBuf,
    artifact_dir: std::path::PathBuf,
    error_log: std::path::PathBuf,
    debug_log: std::path::PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("tmpdir");
    Fixture {
        data_log: dir.path().join("out.jsonl"),
        summary_json: dir.path().join("summary.json"),
        artifact_dir: dir.path().join("artifacts"),
        error_log: dir.path().join("error.log"),
        debug_log: dir.path().join("debug.log"),
        _dir: dir,
    }
}

fn run_imagegen(fixture: &Fixture, base_url: &str, requests: u32, extra: &[&str]) {
    let mut args: Vec<String> = [
        "--url",
        base_url,
        "--api-key",
        "dummy",
        "--scenario",
        "e2e-imagegen",
        "--model",
        "dummy",
        "--num-requests",
        &requests.to_string(),
        "--concurrency",
        "1",
        "--prompt",
        "a small test image",
        "--size",
        "64x64",
        "--data-log",
        fixture.data_log.to_str().unwrap(),
        "--summary-json",
        fixture.summary_json.to_str().unwrap(),
        "--artifact-dir",
        fixture.artifact_dir.to_str().unwrap(),
        "--error-log",
        fixture.error_log.to_str().unwrap(),
        "--debug-log",
        fixture.debug_log.to_str().unwrap(),
        "--no-save-images",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.extend(extra.iter().map(|s| s.to_string()));

    let output = Command::new(imagegen_bin())
        .args(&args)
        .output()
        .expect("run imagegen");
    assert!(
        output.status.success(),
        "imagegen bench failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Headline latency comes from a monotonic clock and tracks the server's
/// configured delay; only shared request.v3 lines land in the data log (N-04).
#[test]
fn imagegen_latency_is_monotonic_and_summary_is_shared() {
    let Some(dummy) = spawn_dummy(&["-latency", "150ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let fixture = fixture();
    run_imagegen(&fixture, &dummy.url("/v1"), 3, &[]);

    let text = std::fs::read_to_string(&fixture.data_log).expect("read data log");
    assert!(
        !text.contains("imagegen.request.v1"),
        "imagegen must not write dual v1 request lines"
    );

    let shared = shared_request_records(&fixture.data_log);
    assert_eq!(
        shared.len(),
        3,
        "expected one shared RequestRecord per request"
    );
    for record in &shared {
        assert!(record.get("error").is_none() || record["error"].is_null());
        let latency_s = record["latency_s"].as_f64().expect("latency_s");
        assert!(
            (0.150..2.0).contains(&latency_s),
            "latency {latency_s}s outside dummy-server bounds"
        );
        assert!(
            record["modality_labels"]["artifact_0_sha256"]
                .as_str()
                .is_some_and(|s| s.len() == 64),
            "artifact sha256 must be stamped on request.v3"
        );
        assert!(record["send_offset_s"].as_f64().is_some());
    }

    let summary = summary_record(&fixture.data_log).expect("shared summary in data log");
    assert_eq!(summary["latency_s"]["n"], 3);
    assert!(
        summary["latency_s"]["p50"].as_f64().expect("p50") >= 0.150,
        "shared summary percentiles disagree with the request records"
    );
    assert_eq!(
        summary["config"]["effective_max_concurrency"], 1,
        "effective_max_concurrency must be stamped"
    );

    let standalone: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&fixture.summary_json).expect("summary json"),
    )
    .expect("parse summary json");
    assert!(
        standalone["schema_version"]
            .as_str()
            .expect("schema")
            .contains("summary.v"),
        "summary-json must be summary.v3, not legacy imagegen.summary.v1"
    );
    assert_eq!(standalone["successes"], 3);
    assert_eq!(standalone["errors"], 0);
    assert_eq!(standalone["latency_s"]["n"], 3);
}

/// Warmup requests are logged but excluded from the measured summary.
#[test]
fn imagegen_warmup_requests_are_excluded_from_summary() {
    let Some(dummy) = spawn_dummy(&[]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let fixture = fixture();
    run_imagegen(&fixture, &dummy.url("/v1"), 3, &["--warmup-requests", "1"]);

    let shared = shared_request_records(&fixture.data_log);
    assert_eq!(shared.len(), 3, "shared RequestRecords must include warmup");
    assert_eq!(shared.iter().filter(|r| r["phase"] == "warmup").count(), 1);
    let summary = summary_record(&fixture.data_log).expect("shared summary");
    assert_eq!(
        summary["latency_s"]["n"], 2,
        "warmup request must not be measured"
    );
}

/// F-22: accept either base URL (`/v1`) or full `/v1/images/generations`.
#[test]
fn imagegen_accepts_full_generations_url() {
    let Some(dummy) = spawn_dummy(&["-latency", "50ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let fixture = fixture();
    run_imagegen(&fixture, &dummy.url("/v1/images/generations"), 2, &[]);

    let shared = shared_request_records(&fixture.data_log);
    assert_eq!(shared.len(), 2);
    for record in &shared {
        assert!(
            record.get("error").is_none() || record["error"].is_null(),
            "full generations URL must work"
        );
    }
}

/// F-28: `--summary-json` is optional; results still land in the data log.
#[test]
fn imagegen_summary_json_is_optional() {
    let Some(dummy) = spawn_dummy(&[]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let fixture = fixture();
    let output = Command::new(imagegen_bin())
        .args([
            "--url",
            &dummy.url("/v1"),
            "--api-key",
            "dummy",
            "--scenario",
            "e2e-imagegen",
            "--model",
            "dummy",
            "--num-requests",
            "2",
            "--concurrency",
            "1",
            "--prompt",
            "a small test image",
            "--size",
            "64x64",
            "--data-log",
            fixture.data_log.to_str().unwrap(),
            "--artifact-dir",
            fixture.artifact_dir.to_str().unwrap(),
            "--error-log",
            fixture.error_log.to_str().unwrap(),
            "--debug-log",
            fixture.debug_log.to_str().unwrap(),
            "--no-save-images",
        ])
        .output()
        .expect("run imagegen");
    assert!(
        output.status.success(),
        "imagegen without --summary-json failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        summary_record(&fixture.data_log).is_some(),
        "summary.v3 must still be in the data log"
    );
}
