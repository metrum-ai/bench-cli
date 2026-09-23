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
    let server = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-mock-server"))
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
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"))
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

#[test]
fn strategic_prompts_and_warmup_exclude_from_aggregates() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve port");
    let address = listener.local_addr().expect("local address");
    drop(listener);
    let server = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-mock-server"))
        .args(["--listen", &address.to_string(), "--latency-ms", "5"])
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
    let prompts = directory.path().join("prompts.jsonl");
    fs::write(
        &prompts,
        "{\"prompt\":\"alpha\"}\n{\"prompt\":\"beta\"}\n{\"prompt\":\"gamma\"}\n",
    )
    .expect("prompts");
    let html = directory.path().join("report.html");
    let csv = directory.path().join("requests.csv");
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"))
        .args([
            "--url",
            &format!("http://{address}/v1/chat/completions"),
            "--model",
            "mock",
            "--prompts",
            prompts.to_str().expect("utf8"),
            "--max-tokens",
            "16",
            "--warmup-requests",
            "2",
            "--requests-per-stage",
            "4",
            "--sweep",
            "1",
            "--html",
            html.to_str().expect("utf8"),
            "--csv",
            csv.to_str().expect("utf8"),
        ])
        .output()
        .expect("run strategic");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let summary: Value = serde_json::from_slice(&output.stdout).expect("summary JSON");
    assert_eq!(summary["points"][0]["n"], 4);
    assert_eq!(summary["points"][0]["config"]["warmup_requests"], 2);
    assert_eq!(summary["points"][0]["config"]["max_tokens"], 16);
    assert_eq!(summary["points"][0]["config"]["prompt_pool_size"], 3);
    let records: Vec<metrum_ai_bench::strategic::BenchRecord> = csv::Reader::from_path(&csv)
        .expect("CSV reader")
        .deserialize()
        .collect::<Result<_, _>>()
        .expect("CSV records");
    assert_eq!(records.len(), 6);
    assert_eq!(records.iter().filter(|r| r.warmup).count(), 2);
    assert_eq!(records.iter().filter(|r| !r.warmup).count(), 4);
}

mod common;

#[test]
fn strategic_sessions_measure_ttft_only_when_streaming() {
    let Some(dummy) = common::spawn_dummy(&["-latency", "20ms", "-chunk-interval", "2ms"]) else {
        common::skip("go dummy-model-server not available");
        return;
    };
    let directory = tempfile::tempdir().expect("temporary directory");
    let sessions = directory.path().join("sessions.jsonl");
    fs::write(&sessions, r#"{"session_id":"s1","messages":[{"role":"user","content":"Hello"},{"role":"user","content":"Again"}]}"#).expect("sessions");
    for streaming in [false, true] {
        let csv = directory.path().join("records.csv");
        let mut command = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"));
        command
            .args([
                "--url",
                &dummy.url("/v1/chat/completions"),
                "--model",
                "dummy",
                "--sweep",
                "1",
                "--requests-per-stage",
                "2",
                "--slo",
                "ttft=0.000001",
            ])
            .arg("--sessions")
            .arg(&sessions)
            .arg("--csv")
            .arg(&csv)
            .arg("--html")
            .arg(directory.path().join("report.html"));
        if streaming {
            command.arg("--streaming");
        }
        let output = command.output().expect("run strategic");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let records: Vec<metrum_ai_bench::strategic::BenchRecord> = csv::Reader::from_path(&csv)
            .expect("CSV")
            .deserialize()
            .collect::<Result<_, _>>()
            .expect("records");
        assert_eq!(records.len(), 2);
        for (index, record) in records.iter().enumerate() {
            assert!(record.success, "{:?}", record.error);
            assert_eq!(record.session_id.as_deref(), Some("s1"));
            assert_eq!(record.turn, Some(index + 1));
            assert!(record.first_byte_s.is_some());
            assert_eq!(record.ttft_s.is_some(), streaming);
            if let Some(ttft) = record.ttft_s {
                assert!(ttft >= 0.015 && ttft <= record.service_latency_s);
                assert!(record.output_tokens > 0);
            }
        }
        let summary: Value = serde_json::from_slice(&output.stdout).expect("summary");
        let goodput = summary["points"][0]["goodput"].as_f64().expect("goodput");
        if streaming {
            assert_eq!(goodput, 0.0);
        } else {
            assert!(goodput > 0.0);
        }
    }
}

#[test]
fn strategic_streaming_rejects_tools_clearly() {
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"))
        .args([
            "--url",
            "http://127.0.0.1:1",
            "--model",
            "dummy",
            "--tools",
            "unused.json",
            "--streaming",
        ])
        .output()
        .expect("run strategic");
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("--tools")
            && error.contains("--streaming")
            && error.contains("cannot be used"),
        "{error}"
    );
}
