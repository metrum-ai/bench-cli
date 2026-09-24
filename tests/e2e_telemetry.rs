// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! E2E: strategic sweep with --telemetry YAML against mock --telemetry-fixture.

use serde_json::Value;
use std::fs;
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn strategic_telemetry_ndjson_scrapes_fixture() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve port");
    let address = listener.local_addr().expect("local address");
    drop(listener);
    let server = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-mock-server"))
        .args([
            "--listen",
            &address.to_string(),
            "--latency-ms",
            "5",
            "--telemetry-fixture",
        ])
        .stdout(Stdio::null())
        .spawn()
        .expect("start mock server");
    let _server = ChildGuard(server);
    let deadline = Instant::now() + Duration::from_secs(10);
    while TcpStream::connect(address).is_err() {
        assert!(Instant::now() < deadline, "mock server did not start");
        thread::sleep(Duration::from_millis(20));
    }

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
    url: http://{address}/metric
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
