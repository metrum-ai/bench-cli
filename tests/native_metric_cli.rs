// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use axum::{routing::get, routing::post, Json, Router};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

fn find_discovery(value: &Value) -> Option<&Value> {
    match value {
        Value::Object(map) => map
            .get("native_cache_metric_discovery")
            .or_else(|| map.values().find_map(find_discovery)),
        Value::Array(values) => values.iter().find_map(find_discovery),
        _ => None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn discovery_is_opt_in_before_measurement_in_both_runners_without_inference_auth() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let metrics = events.clone();
    let chat = events.clone();
    let app=Router::new().route("/metrics",get(move |headers:axum::http::HeaderMap| {
        let events=metrics.clone();async move {
            assert!(headers.get("authorization").is_none());
            events.lock().unwrap().push("metrics");
            "# TYPE vllm:prefix_cache_queries_total counter\nvllm:prefix_cache_queries_total{worker=\"PRIVATE_WORKER\"} 10\n# TYPE vllm:prefix_cache_hits_total counter\nvllm:prefix_cache_hits_total{worker=\"PRIVATE_WORKER\"} 2\n"
        }
    })).route("/chat/completions",post(move || {let events=chat.clone();async move {
        events.lock().unwrap().push("chat");
        Json(json!({"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":8,"completion_tokens":1}}))
    }}));
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    for strategic in [false, true] {
        for enabled in [false, true] {
            events.lock().unwrap().clear();
            let dir = tempfile::tempdir().unwrap();
            let prompts = dir.path().join("prompts.jsonl");
            std::fs::write(&prompts, "{\"prompt\":\"Hi\"}\n").unwrap();
            let log = dir.path().join("requests.jsonl");
            let mut cmd = tokio::process::Command::new(if strategic {
                env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic")
            } else {
                env!("CARGO_BIN_EXE_metrum-ai-bench-cli-llm")
            });
            cmd.args([
                "--url",
                &format!("http://{address}/chat/completions"),
                "--api-key",
                "dummy",
                "--model",
                "mock",
                "--max-tokens",
                "2",
                "--warmup-requests",
                "0",
                "--infer-ttft-from-first-byte",
            ])
            .arg("--prompts")
            .arg(&prompts);
            if enabled {
                cmd.args([
                    "--cache-metrics-url",
                    &format!("http://{address}/metrics"),
                    "--cache-metrics-profile",
                    "vllm-apc",
                    "--cache-metrics-source-revision",
                    metrum_ai_bench::cache_metrics::ProfileId::VllmApc.source_revision(),
                ]);
            }
            if strategic {
                cmd.args(["--sweep", "1", "--requests-per-stage", "1"])
                    .arg("--csv")
                    .arg(dir.path().join("requests.csv"))
                    .arg("--html")
                    .arg(dir.path().join("report.html"));
            } else {
                cmd.args([
                    "--scenario",
                    "native-discovery",
                    "--mode",
                    "chat",
                    "--num-requests",
                    "1",
                    "--concurrency",
                    "1",
                ])
                .arg("--data-log")
                .arg(&log)
                .arg("--debug-log")
                .arg(dir.path().join("debug.log"))
                .arg("--error-log")
                .arg(dir.path().join("error.log"));
            }
            let output = cmd.output().await.unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let rows: Vec<Value> = if strategic {
                vec![serde_json::from_slice(&output.stdout).unwrap()]
            } else {
                std::fs::read_to_string(&log)
                    .unwrap()
                    .lines()
                    .map(|line| serde_json::from_str(line).unwrap())
                    .collect()
            };
            let discovery = rows.iter().find_map(find_discovery);
            assert_eq!(discovery.is_some(), enabled);
            if let Some(report) = discovery {
                assert_eq!(report["schema_version"], "cache-metric-capture.v1");
                assert!(report["failure"].is_null());
                assert_eq!(report["samples"].as_array().unwrap().len(), 2);
                assert_eq!(report["samples"][1]["value"], 2.0);
                assert!(!report.to_string().contains("PRIVATE_WORKER"));
                assert!(!report.to_string().contains(&address.to_string()));
            }
            let observed = events.lock().unwrap().clone();
            assert_eq!(
                observed,
                if enabled {
                    vec!["metrics", "chat"]
                } else {
                    vec!["chat"]
                }
            );
        }
    }
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}
