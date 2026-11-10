// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
#![cfg(unix)]

use axum::{body::Bytes, extract::State, routing::post, Json, Router};
use metrum_ai_bench::cache_workload::sha256;
use serde_json::{json, Value};
use std::process::Command;
use std::sync::{Arc, Mutex};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_busting_preserves_closed_loop_recorded_turns_and_reset_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("reset.marker");
    let bodies = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let observed = bodies.clone();
    let reset_marker = marker.clone();
    let app=Router::new().route("/chat/completions",post(
        move |State(bodies):State<Arc<Mutex<Vec<Vec<u8>>>>>, bytes:Bytes| {
            let marker=reset_marker.clone(); async move {
                let mut list=bodies.lock().unwrap();
                assert_eq!(marker.exists(),list.len()>=2,"reset must follow warmup drain before measurement");
                list.push(bytes.to_vec());
                Json(json!({"choices":[{"message":{"role":"assistant","content":"GENERATED"}}],
                    "usage":{"prompt_tokens":8,"completion_tokens":1}}))
            }
        })).with_state(bodies);
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let sessions = dir.path().join("sessions.jsonl");
    std::fs::write(
        &sessions,
        json!({"session_id":"recorded","messages":[
        {"role":"user","content":"one"},{"role":"assistant","content":"RECORDED"},
        {"role":"user","content":"two"}]})
        .to_string(),
    )
    .unwrap();
    let csv = dir.path().join("requests.csv");
    let hook=format!("touch '{}'; printf '%s' '{{\"success\":true,\"scope\":[\"resident\"],\"ready\":true,\"settled\":true}}'",marker.display());
    let out = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"))
        .args([
            "--url",
            &format!("http://{address}/chat/completions"),
            "--model",
            "mock",
            "--sweep",
            "1",
            "--warmup-requests",
            "2",
            "--requests-per-stage",
            "4",
            "--max-tokens",
            "4",
            "--session-mode",
            "closed-loop",
            "--session-request-roles",
            "user",
            "--cache-policy",
            "bust-request",
            "--prompt-order",
            "preserve",
            "--stage-hook",
            &hook,
            "--stage-reset-scope",
            "resident",
        ])
        .arg("--sessions")
        .arg(&sessions)
        .arg("--csv")
        .arg(&csv)
        .arg("--html")
        .arg(dir.path().join("report.html"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let received = observed.lock().unwrap().clone();
    assert_eq!(received.len(), 6);
    let mut rows: Vec<metrum_ai_bench::strategic::BenchRecord> = csv::Reader::from_path(csv)
        .unwrap()
        .deserialize()
        .collect::<Result<_, _>>()
        .unwrap();
    rows.sort_by_key(|r| r.seq);
    assert_eq!(
        rows.iter().map(|r| r.turn).collect::<Vec<_>>(),
        vec![Some(1), Some(3), Some(1), Some(3), Some(1), Some(3)]
    );
    assert_eq!(rows.iter().filter(|r| r.warmup).count(), 2);
    let mut unique = std::collections::BTreeSet::new();
    for (i, (wire, row)) in received.iter().zip(rows.iter()).enumerate() {
        let hash = sha256(wire);
        assert_eq!(row.body_sha256.as_ref(), Some(&hash));
        unique.insert(hash);
        let body: Value = serde_json::from_slice(wire).unwrap();
        assert_eq!(
            body["messages"].as_array().unwrap().len(),
            if i % 2 == 0 { 1 } else { 3 }
        );
        if i % 2 == 1 {
            assert_eq!(body["messages"][1]["content"], "RECORDED");
        }
        assert!(!String::from_utf8_lossy(wire).contains("GENERATED"));
    }
    assert_eq!(
        unique.len(),
        6,
        "request nonce must remain distinct across warmup and measured cycles"
    );
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}
