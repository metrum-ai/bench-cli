// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

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
fn strategic_sweep_exports_all_formats() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve port");
    let address = listener.local_addr().expect("local address");
    drop(listener);
    let server = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-mock-server"))
        .args(["--listen", &address.to_string(), "--latency-ms", "10"])
        .stdout(Stdio::null())
        .spawn()
        .expect("start mock server");
    let _server = ChildGuard(server);
    let deadline = Instant::now() + Duration::from_secs(10);
    while TcpStream::connect(address).is_err() {
        assert!(Instant::now() < deadline, "mock server did not start");
        thread::sleep(Duration::from_millis(20));
    }

    let directory = tempfile::tempdir().expect("temporary output directory");
    let html = directory.path().join("report.html");
    let csv = directory.path().join("requests.csv");
    let mlperf = directory.path().join("mlperf");
    let schema = directory.path().join("schema.json");
    fs::write(
        &schema,
        r#"{"type":"object","properties":{"answer":{"type":"string"}},"required":["answer"],"additionalProperties":false}"#,
    )
    .expect("write schema");
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-strategic"))
        .args([
            "--url",
            &format!("http://{address}/v1/chat/completions"),
            "--model",
            "mock",
            "--requests-per-stage",
            "3",
            "--sweep",
            "1000,2000,4000",
            "--sweep-by",
            "rate",
            "--max-in-flight",
            "1",
            "--metrics-url",
            &format!("http://{address}/metrics"),
            "--metrics-interval-ms",
            "50",
            "--json-schema",
            schema.to_str().expect("UTF-8 path"),
            "--html",
            html.to_str().expect("UTF-8 path"),
            "--csv",
            csv.to_str().expect("UTF-8 path"),
            "--mlperf-dir",
            mlperf.to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("run strategic benchmark");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let summary: Value = serde_json::from_slice(&output.stdout).expect("summary JSON");
    assert_eq!(summary["points"].as_array().map(Vec::len), Some(3));
    assert!(summary["points"]
        .as_array()
        .expect("points")
        .iter()
        .all(|point| point["validity_rate"] == 1.0));
    assert!(summary["server_metrics"]["kv_cache_usage"].is_number());
    assert!(fs::read_to_string(&html)
        .expect("HTML report")
        .contains("<svg"));
    assert_eq!(
        fs::read_to_string(&csv)
            .expect("CSV records")
            .lines()
            .count(),
        10
    );
    let records: Vec<metrum_ai_bench::strategic::BenchRecord> = csv::Reader::from_path(&csv)
        .expect("CSV reader")
        .deserialize()
        .collect::<Result<_, _>>()
        .expect("CSV records");
    assert!(records
        .iter()
        .skip(1)
        .any(|record| record.queue_delay_s > 0.005));
    assert!(records
        .iter()
        .all(|record| record.latency_s >= record.service_latency_s));
    assert!(mlperf.join("mlperf_log_summary.txt").is_file());
    assert!(mlperf.join("mlperf_log_detail.txt").is_file());
    assert!(mlperf.join("mlperf_log_accuracy.json").is_file());
    let mlperf_summary =
        fs::read_to_string(mlperf.join("mlperf_log_summary.txt")).expect("mlperf summary");
    assert!(mlperf_summary.starts_with("UNOFFICIAL"));
    assert!(mlperf_summary.contains("UNOFFICIAL"));
    assert!(mlperf_summary.contains("Result validity : UNOFFICIAL_OK"));
    assert!(!mlperf_summary.contains("Result is : VALID"));
    assert!(summary["points"]
        .as_array()
        .expect("points")
        .iter()
        .all(|point| point["n"].as_u64().unwrap_or(0) > 0
            && point["latency_s"]["percentile_method"] == "hyndman_fan_type7"
            && point["goodput_equals_throughput"] == true));
}
