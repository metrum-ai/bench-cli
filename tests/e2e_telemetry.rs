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

/// Metrics page that serves `ok` 200 responses, then only 500s.
fn flaky_metrics(ok: usize) -> std::net::SocketAddr {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = listener.local_addr().expect("addr");
    std::thread::spawn(move || {
        for (served, stream) in listener.incoming().enumerate() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let response = if served < ok {
                let body = "all_smi_gpu_utilization{gpu=\"0\"} 50\n";
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            } else {
                "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_string()
            };
            let _ = stream.write_all(response.as_bytes());
        }
    });
    address
}

/// A required source that passes the startup probe and then fails stops the
/// sweep mid-run: non-zero exit, a partial NDJSON summary, fewer stages.
#[test]
fn strategic_require_telemetry_aborts_mid_run() {
    let server = common::spawn_mock(&["--latency-ms", "50"]);
    let address = server.address;
    let metrics = flaky_metrics(1);
    let directory = tempfile::tempdir().expect("tmp");
    let ndjson = directory.path().join("run.ndjson");
    let telemetry = directory.path().join("telemetry.yaml");
    fs::write(
        &telemetry,
        format!(
            "timeout_ms: 500\nsources:\n  - name: flaky\n    url: http://{metrics}/metrics\n    interval_ms: 100\n    include: [\"^all_smi_\"]\n"
        ),
    )
    .expect("write telemetry yaml");
    let stages = ["1", "2", "3", "4", "5", "6"];
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"))
        .args([
            "--url",
            &format!("http://{address}/v1/chat/completions"),
            "--model",
            "mock",
            "--api-key",
            "dummy",
            "--max-tokens",
            "8",
            "--requests-per-stage",
            "20",
            "--sweep",
            &stages.join(","),
            "--ndjson",
            ndjson.to_str().unwrap(),
            "--telemetry",
            telemetry.to_str().unwrap(),
            "--require-telemetry",
            "--html",
            directory.path().join("report.html").to_str().unwrap(),
            "--csv",
            directory.path().join("requests.csv").to_str().unwrap(),
        ])
        .output()
        .expect("run strategic");
    assert!(
        !output.status.success(),
        "a mid-run required-source failure must fail the run; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows: Vec<Value> = fs::read_to_string(&ndjson)
        .expect("ndjson")
        .lines()
        .map(|line| serde_json::from_str(line).expect("ndjson row"))
        .collect();
    assert_eq!(rows[0]["kind"], "run");
    let summary = rows.last().expect("rows");
    assert_eq!(summary["kind"], "summary", "final row is the summary");
    assert_eq!(summary["partial"], true);
    let measured_stages = rows
        .iter()
        .filter(|row| row["kind"] == "stage" && row["phase"] == "measure")
        .count();
    assert!(
        measured_stages < stages.len(),
        "abort must stop the sweep early: {measured_stages} of {} stages",
        stages.len()
    );
    assert!(rows.iter().any(|row| row["kind"] == "scrape_error"));
}
