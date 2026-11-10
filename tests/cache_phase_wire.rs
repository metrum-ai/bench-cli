// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
use axum::{
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
fn analysis(v: &Value) -> Option<&Value> {
    match v {
        Value::Object(m) => m
            .get("native_cache_phase_analysis")
            .or_else(|| m.values().find_map(analysis)),
        Value::Array(a) => a.iter().find_map(analysis),
        _ => None,
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn llm_native_window_replaces_baseline_and_finishes_after_request_drain() {
    let state = Arc::new(Mutex::new((10u64, 2u64, Vec::<String>::new())));
    let metric = state.clone();
    let chat = state.clone();
    let app=Router::new().route("/metrics",get(move |headers:axum::http::HeaderMap| {let state=metric.clone();async move {
        assert!(!headers.contains_key("authorization"));
        let mut s=state.lock().unwrap();
        if let Some(tag)=headers.get("x-metrum-telemetry-boundary").and_then(|v|v.to_str().ok()) {s.2.push(tag.to_owned());}
        format!("# TYPE vllm:prefix_cache_queries_total counter\nvllm:prefix_cache_queries_total{{worker=\"PRIVATE_WORKER\"}} {}\n# TYPE vllm:prefix_cache_hits_total counter\nvllm:prefix_cache_hits_total{{worker=\"PRIVATE_WORKER\"}} {}\n",s.0,s.1)
    }})).route("/chat/completions",post(move || {let state=chat.clone();async move {
        {let mut s=state.lock().unwrap();s.2.push("chat_start".into());}
        tokio::task::yield_now().await;
        {let mut s=state.lock().unwrap();s.0+=8;s.1+=4;s.2.push("chat_complete".into());}
        Json(json!({"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":8,"completion_tokens":1}}))
    }}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let prompts = dir.path().join("prompts.jsonl");
    std::fs::write(&prompts, "{\"prompt\":\"Hi\"}\n").unwrap();
    let yaml = dir.path().join("telemetry.yaml");
    std::fs::write(&yaml,format!("default_interval_ms: 60000\ntimeout_ms: 1000\nsources:\n  - name: engine\n    url: http://{addr}/metrics\n    include: ['^vllm:prefix_cache_.*']\n")).unwrap();
    let log = dir.path().join("requests.jsonl");
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-llm"))
        .args([
            "--url",
            &format!("http://{addr}/chat/completions"),
            "--api-key",
            "dummy",
            "--model",
            "mock",
            "--max-tokens",
            "2",
            "--scenario",
            "phase-window",
            "--mode",
            "chat",
            "--num-requests",
            "3",
            "--warmup-requests",
            "1",
            "--concurrency",
            "2",
            "--infer-ttft-from-first-byte",
            "--cache-metrics-url",
            &format!("http://{addr}/metrics"),
            "--cache-metrics-profile",
            "vllm-apc",
            "--cache-metrics-source-revision",
            metrum_ai_bench::cache_metrics::ProfileId::VllmApc.source_revision(),
            "--cache-phase-source",
            "engine",
            "--cache-phase-epoch",
            "PRIVATE_DECLARATION",
            "--cache-phase-required",
            "--cache-phase-settle-ms",
            "5",
        ])
        .arg("--prompts")
        .arg(&prompts)
        .arg("--telemetry")
        .arg(&yaml)
        .arg("--ndjson")
        .arg(dir.path().join("run.ndjson"))
        .arg("--data-log")
        .arg(&log)
        .arg("--debug-log")
        .arg(dir.path().join("debug.log"))
        .arg("--error-log")
        .arg(dir.path().join("error.log"))
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows: Vec<Value> = std::fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let report = rows
        .iter()
        .find_map(analysis)
        .expect("measured-window report");
    assert_eq!(report["delta"]["phase"], "measured");
    assert_eq!(report["delta"]["continuity"], "operator_declared");
    assert!(report["delta"]["window_unknown"].is_null());
    assert_eq!(report["physical_cache_state"], "unknown");
    assert_eq!(report["sample_freshness"], "unknown");
    let series = report["delta"]["series"].as_array().unwrap();
    assert_eq!(
        series
            .iter()
            .find(|r| r["metric"] == "vllm:prefix_cache_queries_total")
            .unwrap()["delta"],
        16.0
    );
    assert_eq!(
        series
            .iter()
            .find(|r| r["metric"] == "vllm:prefix_cache_hits_total")
            .unwrap()["delta"],
        8.0
    );
    assert!(
        !report.to_string().contains("PRIVATE") && !report.to_string().contains(&addr.to_string())
    );
    let events = state.lock().unwrap().2.clone();
    let before = events
        .iter()
        .position(|e| e == "cache-stage-0-measured-before")
        .unwrap();
    let after = events
        .iter()
        .position(|e| e == "cache-stage-0-measured-after")
        .unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.as_str() == "pre-measured")
            .count(),
        0,
        "no duplicate legacy baseline"
    );
    assert_eq!(
        events[..before]
            .iter()
            .filter(|e| e.as_str() == "chat_complete")
            .count(),
        1,
        "warmup drained before baseline"
    );
    assert_eq!(
        events[before + 1..after]
            .iter()
            .filter(|e| e.as_str() == "chat_complete")
            .count(),
        2
    );
    assert_eq!(
        after,
        events.len() - 1,
        "terminal capture follows all completions"
    );
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}
