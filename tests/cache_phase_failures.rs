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
async fn required_baseline_refuses_issuance_and_optional_failures_never_become_zero() {
    // 0=required HTTP baseline,1=required missing native family,2=required
    // terminal HTTP,3=optional HTTP baseline. Startup/periodic probes succeed.
    let state = Arc::new(Mutex::new((0usize, 0usize)));
    let metric = state.clone();
    let chat = state.clone();
    let app=Router::new().route("/metrics",get(move |headers:axum::http::HeaderMap| {let state=metric.clone();async move {
        assert!(!headers.contains_key("authorization"));let case=state.lock().unwrap().0;
        let tag=headers.get("x-metrum-telemetry-boundary").and_then(|v|v.to_str().ok()).unwrap_or("");
        if ((case==0 || case==3) && tag.ends_with("before")) || (case==2 && tag.ends_with("after")) {return (axum::http::StatusCode::INTERNAL_SERVER_ERROR,"PRIVATE_BODY".to_string());}
        let mut text=String::from("# TYPE vllm:prefix_cache_queries_total counter\nvllm:prefix_cache_queries_total{worker=\"PRIVATE_WORKER\"} 10\n");
        if !(case==1 && tag.ends_with("before")) {text.push_str("# TYPE vllm:prefix_cache_hits_total counter\nvllm:prefix_cache_hits_total{worker=\"PRIVATE_WORKER\"} 0\n");}
        (axum::http::StatusCode::OK,text)
    }})).route("/chat/completions",post(move || {let state=chat.clone();async move {
        state.lock().unwrap().1+=1;
        Json(json!({"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":8,"completion_tokens":1}}))
    }}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    for strategic in [false, true] {
        for case in 0..4 {
            *state.lock().unwrap() = (case, 0);
            let dir = tempfile::tempdir().unwrap();
            let prompts = dir.path().join("prompts.jsonl");
            std::fs::write(&prompts, "{\"prompt\":\"Hi\"}\n").unwrap();
            let yaml = dir.path().join("telemetry.yaml");
            std::fs::write(&yaml,format!("default_interval_ms: 60000\ntimeout_ms: 1000\nsources:\n  - name: engine\n    url: http://{addr}/metrics\n    include: ['^vllm:prefix_cache_.*']\n")).unwrap();
            let log = dir.path().join("requests.jsonl");
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
                "mock",
                "--max-tokens",
                "2",
                "--warmup-requests",
                "0",
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
            ])
            .arg("--prompts")
            .arg(&prompts)
            .arg("--telemetry")
            .arg(&yaml)
            .arg("--ndjson")
            .arg(dir.path().join("run.ndjson"));
            if case != 3 {
                cmd.arg("--cache-phase-required");
            }
            if strategic {
                cmd.args(["--sweep", "1,2", "--requests-per-stage", "2"])
                    .arg("--csv")
                    .arg(dir.path().join("requests.csv"))
                    .arg("--html")
                    .arg(dir.path().join("report.html"));
            } else {
                cmd.args([
                    "--scenario",
                    "phase-failures",
                    "--mode",
                    "chat",
                    "--num-requests",
                    "2",
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
            assert_eq!(
                output.status.success(),
                case == 3,
                "strategic={strategic},case={case}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                state.lock().unwrap().1,
                if case <= 1 {
                    0
                } else if strategic && case == 3 {
                    4
                } else {
                    2
                },
                "required failure must prevent baseline issuance/next stages"
            );
            if case == 3 {
                let values: Vec<Value> = if strategic {
                    vec![serde_json::from_slice(&output.stdout).unwrap()]
                } else {
                    std::fs::read_to_string(&log)
                        .unwrap()
                        .lines()
                        .map(|l| serde_json::from_str(l).unwrap())
                        .collect()
                };
                let report = values.iter().find_map(analysis).unwrap();
                let reports: Vec<&Value> = if strategic {
                    report.as_array().unwrap().iter().collect()
                } else {
                    vec![report]
                };
                for report in reports {
                    assert_eq!(report["physical_cache_state"], "unknown");
                    if let Some(delta) = report.get("delta") {
                        assert_eq!(delta["window_unknown"], "capture_unavailable");
                        assert!(delta["series"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .all(|r| r["delta"].is_null()));
                    } else {
                        assert_eq!(report["unknown"], "capture_unavailable");
                    }
                    assert!(
                        !report.to_string().contains("PRIVATE")
                            && !report.to_string().contains(&addr.to_string())
                    );
                }
            }
        }
    }
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}
