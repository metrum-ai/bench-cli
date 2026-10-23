// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

mod common;

use std::collections::BTreeSet;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

#[test]
fn bench_only_parity_scenario_compares_nonzero_data_points() {
    let mock = common::spawn_mock(&["--token-delay-ms", "1"]);
    let directory = tempfile::tempdir().expect("scenario directory");
    let prompts = directory.path().join("prompts.jsonl");
    let mut prompt_file = std::fs::File::create(&prompts).expect("create prompts");
    writeln!(
        prompt_file,
        r#"{{"prompt":"A short paced parity response."}}"#
    )
    .unwrap();
    let data_log = directory.path().join("parity.jsonl");
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-llm"));
    let status = Command::new(binary)
        .args([
            "--url",
            &url,
            "--api-key",
            "dummy",
            "--scenario",
            "parity-pair",
            "--num-requests",
            "68",
            "--warmup-requests",
            "4",
            "--concurrency",
            "4",
            "--prompts",
            prompts.to_str().unwrap(),
            "--mode",
            "chat",
            "--streaming",
            "--model",
            "mock",
            "--max-tokens",
            "16",
            "--data-log",
            data_log.to_str().unwrap(),
            "--log-level",
            "error",
        ])
        .status()
        .expect("run parity scenario");
    assert!(status.success(), "Bench parity scenario failed");

    let records = common::request_records(&data_log);
    let measured: Vec<_> = records
        .iter()
        .filter(|record| record["phase"] == "measure")
        .collect();
    let fields: BTreeSet<_> = measured
        .iter()
        .flat_map(|record| {
            record
                .as_object()
                .unwrap()
                .iter()
                .filter_map(|(name, value)| {
                    (!value.is_null() && name != "seq" && name != "error").then_some(name.clone())
                })
        })
        .collect();
    assert_eq!(measured.len(), 64, "must compare every measured request");
    assert!(
        !fields.is_empty(),
        "parity scenario compared zero data points"
    );
}

#[tokio::test]
async fn mock_server_replays_recorded_metrics_page() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let fixture = directory.path().join("metrics.prom");
    let expected = "all_smi_gpu_power_consumption_watts 245.0\\n";
    std::fs::write(&fixture, expected).expect("write fixture");
    let fixture_path = fixture.to_str().expect("fixture path is UTF-8");
    let mock = common::spawn_mock(&["--metrics-fixture", fixture_path]);

    let body = reqwest::get(format!("http://{}/metrics", mock.address))
        .await
        .expect("metrics response")
        .text()
        .await
        .expect("metrics body");
    assert_eq!(body, expected);
}

#[tokio::test]
async fn mock_server_paces_streamed_content_chunks() {
    let mock = common::spawn_mock(&["--token-delay-ms", "20"]);
    let started = Instant::now();
    let response = reqwest::Client::new()
        .post(format!("http://{}/v1/chat/completions", mock.address))
        .json(&serde_json::json!({"model":"mock", "stream":true, "messages":[]}))
        .send()
        .await
        .expect("mock response");
    assert!(response.status().is_success());
    let body = response.text().await.expect("stream body");

    assert!(body.contains("data: [DONE]"));
    assert!(
        body.matches("data: {").count() > 1,
        "expected multiple SSE chunks"
    );
    assert!(
        started.elapsed() >= Duration::from_millis(100),
        "SSE chunks were not paced"
    );
}
