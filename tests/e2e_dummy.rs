// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! LLM end-to-end checks against dummy-model-server (Go). Skipped if `go` is missing.

mod common;

use common::{request_records, skip, spawn_dummy};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

fn llm_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_metrum-ai-bench-llm"))
}

fn run_llm_against(url: &str) {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let prompts = tmp.path().join("prompts.jsonl");
    {
        let mut f = std::fs::File::create(&prompts).unwrap();
        writeln!(f, r#"{{"prompt":"Hi"}}"#).unwrap();
    }
    let data_log = tmp.path().join("out.jsonl");
    let status = Command::new(llm_bin())
        .args([
            "--url",
            url,
            "--api-key",
            "dummy",
            "--scenario",
            "e2e",
            "--num-requests",
            "1",
            "--concurrency",
            "1",
            "--prompts",
            prompts.to_str().unwrap(),
            "--mode",
            "chat",
            "--streaming",
            "--model",
            "dummy",
            "--max-tokens",
            "20",
            "--data-log",
            data_log.to_str().unwrap(),
            "--debug-log",
            tmp.path().join("debug.log").to_str().unwrap(),
            "--error-log",
            tmp.path().join("error.log").to_str().unwrap(),
            "--log-level",
            "error",
        ])
        .status()
        .expect("run llm");
    assert!(status.success(), "llm bench failed");

    let records = request_records(&data_log);
    let record = records.last().expect("request record");
    let ttft_ms = record["ttft_s"].as_f64().expect("ttft") * 1000.0;
    let lat_ms = record["latency_s"].as_f64().expect("latency") * 1000.0;
    // Dummy: latency=100ms + first chunk 20ms => TTFT ~120ms; 20 tokens * 20ms + 100ms => ~500ms.
    assert!((100.0..200.0).contains(&ttft_ms), "ttft_ms={ttft_ms}");
    assert!((420.0..650.0).contains(&lat_ms), "latency_ms={lat_ms}");
    let itl: Vec<f64> = record["itl_s"]
        .as_array()
        .map(|arr| arr.iter().filter_map(serde_json::Value::as_f64).collect())
        .unwrap_or_default();
    if !itl.is_empty() {
        let mean = itl.iter().sum::<f64>() / itl.len() as f64;
        assert!(
            (0.010..0.040).contains(&mean),
            "mean ITL {mean}s (want ~0.020s)"
        );
    }
}

#[test]
fn llm_streaming_dummy_timing() {
    let Some(dummy) = spawn_dummy(&["-latency", "100ms", "-chunk-interval", "20ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    run_llm_against(&dummy.url("/v1/chat/completions"));
}

/// F-01 acceptance: window and rps come from in-task send/completion times, not
/// the collector join loop. Dummy @ c=4 / n=16 ≈ 4 waves × ~0.5 s → ~7.9 req/s.
#[test]
fn llm_closed_loop_window_matches_record_span() {
    let Some(dummy) = spawn_dummy(&["-latency", "100ms", "-chunk-interval", "20ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let tmp = tempfile::tempdir().expect("tmpdir");
    let prompts = tmp.path().join("prompts.jsonl");
    {
        let mut f = std::fs::File::create(&prompts).unwrap();
        writeln!(f, r#"{{"prompt":"Hi"}}"#).unwrap();
    }
    let data_log = tmp.path().join("out.jsonl");
    let status = Command::new(llm_bin())
        .args([
            "--url",
            &dummy.url("/v1/chat/completions"),
            "--api-key",
            "dummy",
            "--scenario",
            "window",
            "--num-requests",
            "16",
            "--concurrency",
            "4",
            "--prompts",
            prompts.to_str().unwrap(),
            "--mode",
            "chat",
            "--streaming",
            "--model",
            "dummy",
            "--max-tokens",
            "20",
            "--warmup-requests",
            "0",
            "--seed",
            "7",
            "--data-log",
            data_log.to_str().unwrap(),
            "--debug-log",
            tmp.path().join("debug.log").to_str().unwrap(),
            "--error-log",
            tmp.path().join("error.log").to_str().unwrap(),
            "--log-level",
            "error",
        ])
        .status()
        .expect("run llm");
    assert!(status.success(), "llm bench failed");

    let records = request_records(&data_log);
    assert_eq!(records.len(), 16, "expected 16 request records");
    let mut min_send = f64::INFINITY;
    let mut max_end = f64::NEG_INFINITY;
    for rec in &records {
        let started = parse_rfc3339(rec["started_at"].as_str().expect("started_at"));
        let latency = rec["latency_s"].as_f64().expect("latency_s");
        min_send = min_send.min(started);
        max_end = max_end.max(started + latency);
        assert!(
            rec.get("scheduled_offset_s").is_none_or(|v| v.is_null()),
            "closed-loop must not fake schedule: {rec}"
        );
        let qd = rec
            .get("queue_delay_s")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        assert!(
            qd.abs() < 1e-9,
            "closed-loop queue_delay_s must be 0, got {qd}: {rec}"
        );
    }
    let expected_window = max_end - min_send;
    let summary = common::summary_record(&data_log).expect("summary.v3");
    let window = summary["window_seconds"].as_f64().expect("window_seconds");
    let rps = summary["requests_per_second"]
        .as_f64()
        .expect("requests_per_second");
    assert!(
        (window - expected_window).abs() / expected_window < 0.05,
        "window_seconds={window} expected≈{expected_window}"
    );
    let expected_rps = 16.0 / expected_window;
    assert!(
        (rps - expected_rps).abs() / expected_rps < 0.05,
        "rps={rps} expected≈{expected_rps} (~7.9)"
    );
    assert!(
        (6.5..9.5).contains(&rps),
        "reference band ~7.9 req/s, got {rps}"
    );

    let config = summary.get("config").expect("summary.config");
    let run_id = config["run_id"].as_str().expect("config.run_id");
    assert!(!run_id.is_empty(), "run_id must be non-empty");
    assert_eq!(config["common"]["seed"], 7);
    assert_eq!(config["common"]["warmup_requests"], 0);
    assert_eq!(
        config["effective_system_prompt"],
        "You are a helpful assistant."
    );
    let body = &config["body_template"];
    let body_str = body.to_string();
    assert!(
        body_str.contains("{{prompt}}"),
        "body_template should use prompt placeholder: {body_str}"
    );
    assert!(
        !body_str.contains("\"Hi\""),
        "body_template must not embed the raw prompt"
    );
    assert!(summary["completion_tokens_per_second"].as_f64().is_some());
    assert_eq!(summary["completion_tokens_source"], "server_usage");
    assert_eq!(summary["usage_missing_count"], 0);
    for rec in &records {
        assert_eq!(rec["run_id"].as_str(), Some(run_id));
    }
}

/// F-04: completing tasks write request.v3 immediately, before launch finishes.
#[test]
fn llm_flushes_request_records_during_launch() {
    let Some(dummy) = spawn_dummy(&["-latency", "200ms", "-chunk-interval", "50ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let tmp = tempfile::tempdir().expect("tmpdir");
    let prompts = tmp.path().join("prompts.jsonl");
    {
        let mut f = std::fs::File::create(&prompts).unwrap();
        writeln!(f, r#"{{"prompt":"Hi"}}"#).unwrap();
    }
    let data_log = tmp.path().join("out.jsonl");
    let mut child = Command::new(llm_bin())
        .args([
            "--url",
            &dummy.url("/v1/chat/completions"),
            "--api-key",
            "dummy",
            "--scenario",
            "flush",
            "--num-requests",
            "32",
            "--concurrency",
            "2",
            "--prompts",
            prompts.to_str().unwrap(),
            "--mode",
            "chat",
            "--streaming",
            "--model",
            "dummy",
            "--max-tokens",
            "8",
            "--warmup-requests",
            "0",
            "--data-log",
            data_log.to_str().unwrap(),
            "--debug-log",
            tmp.path().join("debug.log").to_str().unwrap(),
            "--error-log",
            tmp.path().join("error.log").to_str().unwrap(),
            "--log-level",
            "error",
        ])
        .spawn()
        .expect("spawn llm");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut saw_request = false;
    while std::time::Instant::now() < deadline {
        if let Ok(Some(status)) = child.try_wait() {
            panic!("llm exited before flush observation: {status}");
        }
        if data_log.exists() {
            if let Ok(text) = std::fs::read_to_string(&data_log) {
                if text.lines().any(|line| {
                    serde_json::from_str::<serde_json::Value>(line)
                        .ok()
                        .and_then(|v| {
                            v.get("schema_version")
                                .and_then(|s| s.as_str())
                                .map(|s| s.contains("request.v"))
                        })
                        .unwrap_or(false)
                }) {
                    saw_request = true;
                    break;
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(
        saw_request,
        "expected a request.v3 line while launch was still running"
    );
    // Still running ⇒ launch/drain not finished when the first record landed.
    assert!(
        child.try_wait().ok().flatten().is_none(),
        "process should still be alive after first flush"
    );
    let _ = child.kill();
    let _ = child.wait();
}

/// SIGTERM stops issuance; JSONL prefix remains line-parseable (truncated tail ok).
#[cfg(unix)]
#[test]
fn llm_sigterm_leaves_parseable_jsonl_prefix() {
    let Some(dummy) = spawn_dummy(&["-latency", "300ms", "-chunk-interval", "40ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let tmp = tempfile::tempdir().expect("tmpdir");
    let prompts = tmp.path().join("prompts.jsonl");
    {
        let mut f = std::fs::File::create(&prompts).unwrap();
        writeln!(f, r#"{{"prompt":"Hi"}}"#).unwrap();
    }
    let data_log = tmp.path().join("out.jsonl");
    let mut child = Command::new(llm_bin())
        .args([
            "--url",
            &dummy.url("/v1/chat/completions"),
            "--api-key",
            "dummy",
            "--scenario",
            "sigterm",
            "--num-requests",
            "64",
            "--concurrency",
            "4",
            "--prompts",
            prompts.to_str().unwrap(),
            "--mode",
            "chat",
            "--streaming",
            "--model",
            "dummy",
            "--max-tokens",
            "16",
            "--warmup-requests",
            "0",
            "--data-log",
            data_log.to_str().unwrap(),
            "--debug-log",
            tmp.path().join("debug.log").to_str().unwrap(),
            "--error-log",
            tmp.path().join("error.log").to_str().unwrap(),
            "--log-level",
            "error",
        ])
        .spawn()
        .expect("spawn llm");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        if data_log.exists() {
            if let Ok(text) = std::fs::read_to_string(&data_log) {
                let n = text
                    .lines()
                    .filter(|line| {
                        serde_json::from_str::<serde_json::Value>(line)
                            .ok()
                            .and_then(|v| {
                                v.get("schema_version")
                                    .and_then(|s| s.as_str())
                                    .map(|s| s.contains("request.v"))
                            })
                            .unwrap_or(false)
                    })
                    .count();
                if n >= 2 {
                    break;
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    let _ = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status();
    let _ = child.wait();

    let text = std::fs::read_to_string(&data_log).expect("read data log");
    let mut parsed = 0usize;
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(_) => parsed += 1,
            Err(_) => {
                // Truncated final line is allowed; everything before must parse.
                break;
            }
        }
    }
    assert!(
        parsed >= 1,
        "expected at least one parseable JSONL line after SIGTERM, got {parsed}"
    );
}

fn parse_rfc3339(s: &str) -> f64 {
    use chrono::{DateTime, Utc};
    let dt: DateTime<Utc> = s.parse().expect("rfc3339");
    dt.timestamp() as f64 + f64::from(dt.timestamp_subsec_nanos()) / 1e9
}

/// Unique prompts stamp run_id+seed+seq into the nonce and record the template.
#[test]
fn llm_unique_prompts_include_run_id_and_seed() {
    let Some(dummy) = spawn_dummy(&["-latency", "40ms", "-chunk-interval", "10ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let tmp = tempfile::tempdir().expect("tmpdir");
    let prompts = tmp.path().join("prompts.jsonl");
    {
        let mut f = std::fs::File::create(&prompts).unwrap();
        writeln!(f, r#"{{"prompt":"Second prompt"}}"#).unwrap();
    }
    let data_log = tmp.path().join("out.jsonl");
    let status = Command::new(llm_bin())
        .args([
            "--url",
            &dummy.url("/v1/chat/completions"),
            "--api-key",
            "dummy",
            "--scenario",
            "unique",
            "--num-requests",
            "4",
            "--concurrency",
            "2",
            "--prompts",
            prompts.to_str().unwrap(),
            "--mode",
            "chat",
            "--streaming",
            "--model",
            "dummy",
            "--max-tokens",
            "5",
            "--warmup-requests",
            "0",
            "--seed",
            "7",
            "--unique-prompts",
            "--data-log",
            data_log.to_str().unwrap(),
            "--debug-log",
            tmp.path().join("debug.log").to_str().unwrap(),
            "--error-log",
            tmp.path().join("error.log").to_str().unwrap(),
            "--log-level",
            "error",
        ])
        .status()
        .expect("run llm");
    assert!(status.success(), "llm bench failed");

    let summary = common::summary_record(&data_log).expect("summary.v3");
    let config = summary.get("config").expect("config");
    let run_id = config["run_id"].as_str().expect("run_id");
    assert_eq!(
        config["unique_prompt_nonce_template"],
        "[nonce-{run_id}-{seed}-{seq}]"
    );
    assert_eq!(config["common"]["unique_prompts"].as_bool(), Some(true));
    assert_eq!(config["common"]["seed"].as_u64(), Some(7));

    // Dummy captures last request bodies; at least the summary template is enough
    // for schema honesty. Nonce shape is covered by unit tests + config stamp.
    assert!(!run_id.is_empty());
    let records = request_records(&data_log);
    assert_eq!(records.len(), 4);
    for rec in &records {
        assert_eq!(rec["run_id"].as_str(), Some(run_id));
    }
}

/// F-08: non-streaming LLM must not fabricate TTFT (null like VLM).
#[test]
fn llm_non_streaming_reports_null_ttft() {
    let Some(dummy) = spawn_dummy(&["-latency", "50ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let tmp = tempfile::tempdir().expect("tmpdir");
    let prompts = tmp.path().join("prompts.jsonl");
    {
        let mut f = std::fs::File::create(&prompts).unwrap();
        writeln!(f, r#"{{"prompt":"Hi"}}"#).unwrap();
    }
    let data_log = tmp.path().join("out.jsonl");
    let status = Command::new(llm_bin())
        .args([
            "--url",
            &dummy.url("/v1/chat/completions"),
            "--api-key",
            "dummy",
            "--scenario",
            "nonstream",
            "--num-requests",
            "2",
            "--concurrency",
            "1",
            "--prompts",
            prompts.to_str().unwrap(),
            "--mode",
            "chat",
            "--model",
            "dummy",
            "--max-tokens",
            "16",
            "--data-log",
            data_log.to_str().unwrap(),
            "--debug-log",
            tmp.path().join("debug.log").to_str().unwrap(),
            "--error-log",
            tmp.path().join("error.log").to_str().unwrap(),
            "--log-level",
            "error",
        ])
        .status()
        .expect("run llm");
    assert!(status.success(), "llm bench failed");

    let records = request_records(&data_log);
    assert_eq!(records.len(), 2);
    for record in &records {
        assert!(
            record["ttft_s"].is_null(),
            "non-streaming record reported ttft {:?}",
            record["ttft_s"]
        );
        assert!(record["latency_s"].as_f64().expect("latency_s") >= 0.050);
    }
    let summary = common::summary_record(&data_log).expect("summary");
    assert_eq!(summary["ttft_s"]["n"], 0);
}
