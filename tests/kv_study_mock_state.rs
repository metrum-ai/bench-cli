// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
//! An explicitly synthetic cache state machine; not engine/tier certification.
use axum::{routing::post, Json, Router};
use metrum_ai_bench::{kv_study_export::*, strategic::BenchRecord};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
#[derive(Default)]
struct MockCache {
    phase: usize,
    queries: usize,
    resident: bool,
    external: bool,
    events: Vec<&'static str>,
}
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
    let phases:Vec<_>=["cold_populate","resident_warm","pressure","rehit","rehit","cold_populate"].iter().enumerate().map(|(i,kind)|json!({"id":format!("fixture-{i}"),"kind":kind,"cache_state":"unknown","reset":{"scope":[],"acknowledged":false,"ready":false,"settled":false,"status":if i==5 {"failed"}else{"not_requested"},"reason":if i==5 {Some("fixture hook success=false")}else{None},"started_unix_ns":null,"acknowledged_unix_ns":null,"ready_unix_ns":null,"settled_unix_ns":null},"requests":999,"errors":0,"cached_token_known":999,"cached_token_unknown":0})).collect();
    json!({"schema_version":"kv-study-report.v1","study_id":"mock-state-only","reuse_kind":"exact_prefix","runtime":runtime,"datasets":[],"phases":phases,"limitations":["Mock cache states certify fixture behavior only, not physical state."],"workload":{"cache_policy":"reuse","order":"preserve","effective_order_sha256":null,"body_fingerprints_sha256":null,"history_mode":"recorded","arrival_mode":"closed_loop","concurrency":1,"input_distribution":null,"output_distribution":null,"warmup_isolation":"unverified"}})
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_http_mock_transitions_and_failed_reset_reconcile_all_export_views_without_physical_claims(
) {
    let cache = Arc::new(Mutex::new(MockCache::default()));
    let state = cache.clone();
    let app=Router::new().route("/chat/completions",post(move |Json(body):Json<Value>|{let state=state.clone();async move {
        let mut cache=state.lock().unwrap();cache.queries+=1;
        let prompt=body["messages"].as_array().unwrap().iter().find(|m|m["role"]=="user").unwrap()["content"].as_str().unwrap();
        assert_eq!(prompt,if cache.phase==2 {"B"}else{"A"});
        let cached=match cache.phase {
            0=>{assert!(!cache.resident && !cache.external);cache.resident=true;cache.events.push("cold_miss");0},
            1=>{assert!(cache.resident);cache.events.push("resident_hit");8},
            2=>{assert!(cache.resident);cache.resident=false;cache.external=true;cache.events.extend(["eviction","pressure_recompute"]);0},
            3=>{assert!(!cache.resident && cache.external);cache.resident=true;cache.events.push("external_hit");8},
            4=>{cache.resident=false;cache.external=false;cache.events.push("external_eviction");assert!(!cache.external);cache.events.push("external_miss_recompute");cache.resident=true;0},
            _=>panic!("failed reset must prevent model requests"),
        };
        Json(json!({"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":8,"completion_tokens":1,"prompt_tokens_details":{"cached_tokens":cached}}}))
    }}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let mut populations = Vec::new();
    for phase in 0..6 {
        cache.lock().unwrap().phase = phase;
        let prompts = dir.path().join(format!("prompts-{phase}.jsonl"));
        std::fs::write(
            &prompts,
            serde_json::to_string(&json!({"prompt":if phase==2 {"B"}else{"A"}})).unwrap() + "\n",
        )
        .unwrap();
        let csv = dir.path().join(format!("requests-{phase}.csv"));
        let html = dir.path().join(format!("report-{phase}.html"));
        let mut cmd =
            tokio::process::Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"));
        cmd.args([
            "--url",
            &format!("http://{addr}/chat/completions"),
            "--api-key",
            "dummy",
            "--model",
            "fixture",
            "--sweep",
            "1",
            "--requests-per-stage",
            "1",
            "--warmup-requests",
            "0",
            "--max-tokens",
            "2",
            "--cache-policy",
            "reuse",
            "--infer-ttft-from-first-byte",
        ])
        .arg("--prompts")
        .arg(&prompts)
        .arg("--csv")
        .arg(&csv)
        .arg("--html")
        .arg(&html);
        if phase == 5 {
            cmd.args(["--stage-hook","printf '%s' '{\"success\":false,\"scope\":[\"resident\",\"external\"],\"ready\":true,\"settled\":true}'","--stage-reset-scope","resident,external"]);
        }
        let output = cmd.output().await.unwrap();
        assert_eq!(
            output.status.success(),
            phase != 5,
            "phase{phase}:{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let rows: Vec<BenchRecord> = if csv.exists() {
            csv::Reader::from_path(&csv)
                .unwrap()
                .deserialize()
                .map(Result::unwrap)
                .collect()
        } else {
            Vec::new()
        };
        assert_eq!(rows.len(), usize::from(phase != 5));
        populations.push(rows);
    }
    assert_eq!(cache.lock().unwrap().queries, 5);
    assert_eq!(
        cache.lock().unwrap().events,
        vec![
            "cold_miss",
            "resident_hit",
            "eviction",
            "pressure_recompute",
            "external_hit",
            "external_eviction",
            "external_miss_recompute"
        ]
    );
    let bindings: BTreeMap<_, _> = populations
        .iter()
        .enumerate()
        .map(|(i, rows)| (format!("fixture-{i}"), PhaseRecords::Strategic(rows)))
        .collect();
    let export = StudyExport::bind(
        StudySeed::parse(&serde_json::to_vec(&seed()).unwrap()).unwrap(),
        &bindings,
    )
    .unwrap();
    let data: Value = serde_json::from_slice(&export.json().unwrap()).unwrap();
    for (i, phase) in data["phases"].as_array().unwrap().iter().enumerate() {
        assert_eq!(phase["requests"], usize::from(i != 5));
        assert_eq!(phase["cached_token_known"], usize::from(i != 5));
        assert_eq!(phase["cache_state"], "unknown");
        assert_eq!(phase["small_n_warning"], true);
    }
    let cold = &data["phases"][0]["cohorts"]["counts"];
    assert_eq!(cold["cold"], 1);
    assert_eq!(
        data["phases"][3]["cohorts"]["counts"]["full"], 1,
        "fixture reports reuse but cannot certify external participation"
    );
    assert_eq!(data["report"]["phases"][5]["reset"]["status"], "failed");
    assert_eq!(data["report"]["phases"][5]["requests"], 0);
    let bytes = export.csv().unwrap();
    let mut reader = csv::Reader::from_reader(bytes.as_slice());
    let headers = reader.headers().unwrap().clone();
    let records: Vec<_> = reader.records().map(Result::unwrap).collect();
    assert_eq!(records.len(), 6);
    for (i, row) in records.iter().enumerate() {
        let column = |name: &str| {
            row.get(headers.iter().position(|s| s == name).unwrap())
                .unwrap()
        };
        assert_eq!(column("requests"), usize::from(i != 5).to_string());
        let cohorts: Value = serde_json::from_str(column("cohorts_json")).unwrap();
        assert_eq!(cohorts, data["phases"][i]["cohorts"]);
    }
    let html = export.html().unwrap();
    assert!(html.contains("Unknown physical state is not cold"));
    assert!(html.contains("Small-n"));
    assert!(!data["publication_blockers"].as_array().unwrap().is_empty());
    server.abort();
}
