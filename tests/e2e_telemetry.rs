// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! E2E: strategic sweep with --telemetry YAML against mock --telemetry-fixture.

mod common;

use serde_json::Value;
use std::fs;
use std::process::Command;

#[test]
fn strategic_telemetry_ndjson_scrapes_fixture() {
    let server = common::spawn_mock(&["--latency-ms", "5", "--telemetry-fixture"]);
    let address = server.address;

    let directory = tempfile::tempdir().expect("tmp");
    let ndjson = directory.path().join("run.ndjson");
    let html = directory.path().join("report.html");
    let csv = directory.path().join("requests.csv");
    let telemetry = directory.path().join("telemetry.yaml");
    fs::write(
        &telemetry,
        format!(
            r#"
default_interval_ms: 1000
timeout_ms: 500
sources:
  - name: all-smi
    url: http://{address}/metrics
    interval_ms: 100
    include:
      - "^all_smi_(gpu|cpu|memory)_"
  - name: dcgm
    url: http://{address}/metrics
    interval_ms: 100
    include:
      - "^DCGM_FI_DEV_(POWER_USAGE|TOTAL_ENERGY_CONSUMPTION|GPU_UTIL)$"
  - name: vllm
    url: http://{address}/metrics
    interval_ms: 100
    include:
      - "^vllm:(gpu_cache_usage_perc|num_requests_(running|waiting)|num_preemptions_total)$"
"#
        ),
    )
    .expect("write telemetry yaml");

    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"))
        .args([
            "--url",
            &format!("http://{address}/v1/chat/completions"),
            "--model",
            "mock",
            "--api-key",
            "dummy",
            "--requests-per-stage",
            "4",
            "--warmup-requests",
            "1",
            "--sweep",
            "1,2",
            "--ndjson",
            ndjson.to_str().unwrap(),
            "--telemetry",
            telemetry.to_str().unwrap(),
            "--require-telemetry",
            "--html",
            html.to_str().unwrap(),
            "--csv",
            csv.to_str().unwrap(),
        ])
        .output()
        .expect("run strategic");
    assert!(
        output.status.success(),
        "stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );

    let text = fs::read_to_string(&ndjson).expect("read ndjson");
    let mut kinds = std::collections::BTreeMap::<String, u64>::new();
    for line in text.lines() {
        let value: Value = serde_json::from_str(line).expect("row json");
        let kind = value["kind"].as_str().expect("kind").to_string();
        *kinds.entry(kind).or_default() += 1;
    }
    assert!(kinds.get("run").copied().unwrap_or(0) >= 1);
    assert!(kinds.get("request").copied().unwrap_or(0) >= 5);
    assert!(kinds.get("stage").copied().unwrap_or(0) >= 2);
    assert!(
        kinds.get("telemetry").copied().unwrap_or(0) >= 3,
        "expected telemetry samples, kinds={kinds:?}"
    );
    assert_eq!(kinds.get("summary").copied().unwrap_or(0), 1);
    let summary: Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    assert_eq!(summary["kind"], "summary");
    assert_eq!(summary["partial"], false);
    assert_eq!(summary["dropped_telemetry_rows"], 0);
}

/// #227: a required source that serves one 200 (the startup probe) and then
/// 500s aborts a strategic sweep mid-run. The run exits non-zero, the NDJSON
/// still ends with a `summary` row marked `partial`, and fewer stages than
/// configured were measured.
#[test]
fn strategic_require_telemetry_aborts_mid_run() {
    const STAGES: usize = 5;
    // 8 requests at 300 ms keep each low-concurrency stage well past the
    // 200 ms the two failing 100 ms scrapes take.
    let server = common::spawn_mock(&["--latency-ms", "300"]);
    let source = common::spawn_flaky_metrics(1);
    let directory = tempfile::tempdir().expect("tmp");
    let ndjson = directory.path().join("run.ndjson");
    let telemetry = common::flaky_telemetry_yaml(directory.path(), source);

    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"))
        .args([
            "--url",
            &format!("http://{}/v1/chat/completions", server.address),
            "--model",
            "mock",
            "--api-key",
            "dummy",
            "--requests-per-stage",
            "8",
            "--warmup-requests",
            "0",
            "--sweep",
            "1,2,4,8,16",
            "--sweep-by",
            "concurrency",
            "--ndjson",
            ndjson.to_str().unwrap(),
            "--telemetry",
            telemetry.to_str().unwrap(),
            "--require-telemetry",
            "--require-telemetry-failures",
            "2",
        ])
        .output()
        .expect("run strategic");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "abort must fail the run: {stderr}"
    );
    assert!(
        stderr.contains("failed 2 consecutive scrapes (require-telemetry)"),
        "{stderr}"
    );

    let rows = common::ndjson_rows(&ndjson);
    assert_eq!(rows.first().map(|r| &r["kind"]), Some(&Value::from("run")));
    let summary = rows.last().expect("ndjson rows");
    assert_eq!(summary["kind"], "summary", "{summary}");
    assert_eq!(summary["partial"], true, "{summary}");
    assert!(
        summary["scrape_error_rows"].as_u64().unwrap_or(0) >= 2,
        "{summary}"
    );
    let measured: std::collections::BTreeSet<u64> = rows
        .iter()
        .filter(|r| r["kind"] == "stage" && r["phase"] == "measure")
        .filter_map(|r| r["stage"].as_f64())
        .map(|stage| stage as u64)
        .collect();
    // The first stage (8 requests at 300 ms, concurrency 1) drains after the
    // abort; stages are numbered from 1.
    assert!(measured.contains(&1), "stage rows: {measured:?}");
    assert!(
        measured.len() < STAGES,
        "expected fewer than {STAGES} measured stages, got {measured:?}"
    );
    let stage_rows = rows.iter().filter(|r| r["kind"] == "stage").count() as u64;
    assert_eq!(summary["stage_rows"], stage_rows, "{summary}");
}
