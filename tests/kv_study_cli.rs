// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
use axum::{
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
fn seed() -> Value {
    let mut runtime = json!({});
    for key in [
        "model_revision",
        "tokenizer_revision",
        "chat_template_sha256",
        "engine",
        "engine_version",
        "connector",
        "connector_version",
        "flags_sha256",
        "block_size_tokens",
        "kv_dtype",
        "kv_layout",
        "tensor_parallelism",
        "routing",
    ] {
        runtime[key] = Value::Null;
    }
    runtime["tiers"] = json!([]);
    json!({"schema_version":"kv-study-report.v1","study_id":"cli-fixture","reuse_kind":"exact_prefix","runtime":runtime,"datasets":[],"limitations":["fixture only"],"workload":{"cache_policy":"reuse","order":"preserve","effective_order_sha256":null,"body_fingerprints_sha256":null,"history_mode":"recorded","arrival_mode":"closed_loop","concurrency":1,"input_distribution":null,"output_distribution":null,"warmup_isolation":"unverified"},"phases":[{"id":"measure","kind":"resident_warm","cache_state":"unknown","reset":{"scope":[],"acknowledged":false,"ready":false,"settled":false,"status":"not_requested","reason":null,"started_unix_ns":null,"acknowledged_unix_ns":null,"ready_unix_ns":null,"settled_unix_ns":null},"requests":0,"errors":0,"cached_token_known":0,"cached_token_unknown":0}]})
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn both_cli_exports_actual_rows_and_refuse_claims_or_overwrites_before_http() {
    let count = Arc::new(AtomicUsize::new(0));
    let hits = count.clone();
    let app=Router::new().route("/chat/completions",post(move||{let hits=hits.clone();async move {hits.fetch_add(1,Ordering::SeqCst);Json(json!({"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":8,"completion_tokens":1,"prompt_tokens_details":{"cached_tokens":0}}}))}}));
    let app=app.route("/metrics",get(|headers:axum::http::HeaderMap|async move {
        let after=headers.get("x-metrum-telemetry-boundary").and_then(|h|h.to_str().ok()).is_some_and(|h|h.ends_with("after"));
        format!("# TYPE vllm:prefix_cache_queries_total counter\nvllm:prefix_cache_queries_total{{worker=\"PRIVATE_WORKER\"}} {}\n# TYPE vllm:prefix_cache_hits_total counter\nvllm:prefix_cache_hits_total{{worker=\"PRIVATE_WORKER\"}} {}\n",if after {12}else{10},usize::from(after))
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    for strategic in [false, true] {
        for case in 0..6 {
            let dir = tempfile::tempdir().unwrap();
            let prompts = dir.path().join("prompts.jsonl");
            std::fs::write(&prompts, "{\"prompt\":\"Hi\"}\n").unwrap();
            let manifest = dir.path().join("seed.json");
            let mut declaration = seed();
            if case == 1 {
                declaration["phases"][0]["cache_state"] = json!("observed_resident");
            }
            if case == 3 {
                declaration["workload"]["cache_policy"] = json!("bust_request");
            }
            if case == 4 {
                declaration["workload"]["order"] = json!("seeded_shuffle");
            }
            std::fs::write(&manifest, serde_json::to_vec(&declaration).unwrap()).unwrap();
            let prefix = dir.path().join("export");
            let json_path = dir.path().join("export.study.json");
            let csv_path = dir.path().join("export.study.csv");
            let html_path = dir.path().join("export.study.html");
            if case == 2 {
                std::fs::write(&csv_path, "SENTINEL").unwrap();
            }
            let mut cmd = tokio::process::Command::new(if strategic {
                env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic")
            } else {
                env!("CARGO_BIN_EXE_metrum-ai-bench-cli-llm")
            });
            cmd.args([
                "--url",
                &format!("http://{addr}/chat/completions"),
                "--api-key",
                "dummy",
                "--model",
                "fixture",
                "--max-tokens",
                "2",
                "--warmup-requests",
                "0",
                "--cache-policy",
                "reuse",
                "--prompt-order",
                "preserve",
                "--infer-ttft-from-first-byte",
            ])
            .arg("--prompts")
            .arg(&prompts)
            .arg("--kv-study-manifest")
            .arg(&manifest)
            .arg("--kv-study-prefix")
            .arg(&prefix);
            if strategic {
                cmd.args(["--sweep", "1", "--requests-per-stage", "1"])
                    .arg("--csv")
                    .arg(dir.path().join("requests.csv"))
                    .arg("--html")
                    .arg(dir.path().join("report.html"));
            } else {
                cmd.args([
                    "--scenario",
                    "study",
                    "--mode",
                    "chat",
                    "--num-requests",
                    "1",
                    "--concurrency",
                    "1",
                ])
                .arg("--data-log")
                .arg(dir.path().join("requests.jsonl"))
                .arg("--debug-log")
                .arg(dir.path().join("debug.log"))
                .arg("--error-log")
                .arg(dir.path().join("error.log"));
            }
            if case == 5 {
                let telemetry = dir.path().join("telemetry.yaml");
                std::fs::write(&telemetry,format!("default_interval_ms: 60000\ntimeout_ms: 1000\nsources:\n  - name: engine\n    url: http://{addr}/metrics\n    include: ['^vllm:prefix_cache_.*']\n")).unwrap();
                cmd.arg("--telemetry")
                    .arg(telemetry)
                    .arg("--ndjson")
                    .arg(dir.path().join("run.ndjson"))
                    .args([
                        "--cache-metrics-url",
                        &format!("http://{addr}/metrics"),
                        "--cache-metrics-profile",
                        "vllm-apc",
                        "--cache-metrics-source-revision",
                        metrum_ai_bench::cache_metrics::ProfileId::VllmApc.source_revision(),
                        "--cache-phase-source",
                        "engine",
                        "--cache-phase-epoch",
                        "PRIVATE_EPOCH",
                    ]);
            }
            let success = case == 0 || case == 5;
            let before = count.load(Ordering::SeqCst);
            let output = cmd.output().await.unwrap();
            assert_eq!(
                output.status.success(),
                success,
                "strategic{strategic}case{case}:{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(count.load(Ordering::SeqCst) - before, usize::from(success));
            if success {
                let data: Value =
                    serde_json::from_slice(&std::fs::read(&json_path).unwrap()).unwrap();
                assert_eq!(data["schema_version"], "kv-study-export.v1");
                if case == 5 {
                    assert!(data["phases"][0]["native_window"]["delta"]["series"].is_array());
                    assert!(!serde_json::to_string(&data).unwrap().contains("PRIVATE"));
                } else {
                    assert_eq!(
                        data["phases"][0]["native_window"]["unknown"],
                        "not_requested_or_not_executed"
                    );
                }
                assert_eq!(data["phases"][0]["requests"], 1);
                assert_eq!(data["phases"][0]["cached_token_known"], 1);
                assert_eq!(data["phases"][0]["cache_state"], "unknown");
                assert_eq!(data["phases"][0]["small_n_warning"], true);
                let mut csv = csv::Reader::from_path(&csv_path).unwrap();
                assert_eq!(csv.records().count(), 1);
                assert!(std::fs::read_to_string(&html_path)
                    .unwrap()
                    .contains("Unknown physical state is not cold"));
            } else {
                assert!(!json_path.exists() && !html_path.exists());
                if case == 2 {
                    assert_eq!(std::fs::read_to_string(&csv_path).unwrap(), "SENTINEL");
                }
            }
        }
    }
    server.abort();
}
