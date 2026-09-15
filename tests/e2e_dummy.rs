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
