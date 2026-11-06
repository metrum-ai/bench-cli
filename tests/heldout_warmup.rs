// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::process::Command;
use std::sync::{Arc, Mutex};

use axum::{body::Bytes, extract::State, routing::post, Json, Router};
use metrum_ai_bench::warmup_isolation::body_digest;
use serde_json::{json, Value};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_isolation_stops_before_traffic_and_override_binds_exact_sent_bodies() {
    let bodies = Arc::new(Mutex::new(Vec::<String>::new()));
    let app = Router::new()
        .route(
            "/chat/completions",
            post(
                |State(bodies): State<Arc<Mutex<Vec<String>>>>, bytes: Bytes| async move {
                    bodies.lock().unwrap().push(body_digest(&bytes));
                    Json(
                        json!({"choices":[{"message":{"role":"assistant","content":"ok"}}],
            "usage":{"prompt_tokens":4,"completion_tokens":1}}),
                    )
                },
            ),
        )
        .with_state(bodies.clone());
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let measured = dir.path().join("measured.jsonl");
    let warmup = dir.path().join("warmup.jsonl");
    std::fs::write(&measured, json!({"session_id":"measured", "messages":[
        {"role":"system","content":"measured-root"},{"role":"user","content":"measured-question"}
    ]}).to_string()).unwrap();
    std::fs::write(
        &warmup,
        json!({"session_id":"held-out", "messages":[
            {"role":"system","content":"warmup-root"},{"role":"user","content":"warmup-question"}
        ]})
        .to_string(),
    )
    .unwrap();
    let url = format!("http://{address}/chat/completions");
    for allow in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"));
        command
            .args([
                "--url",
                &url,
                "--model",
                "mock",
                "--sweep",
                "1",
                "--requests-per-stage",
                "2",
                "--warmup-requests",
                "1",
                "--max-tokens",
                "4",
                "--session-request-roles",
                "user",
            ])
            .arg("--sessions")
            .arg(&measured)
            .arg("--warmup-sessions")
            .arg(&warmup)
            .arg("--html")
            .arg(dir.path().join("report.html"));
        if allow {
            command.arg("--allow-unverified-warmup");
        }
        let output = command.output().unwrap();
        if !allow {
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("Unknown"));
            assert!(bodies.lock().unwrap().is_empty());
        } else {
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let summary: Value = serde_json::from_slice(&output.stdout).unwrap();
            let metadata = &summary["config"]["warmup"];
            assert_eq!(metadata["override_unverified"], true);
            assert_eq!(metadata["overlap_report"]["verdict"], "Unknown");
            assert_eq!(metadata["rendered_tokens_available"], false);
            let sent = bodies.lock().unwrap();
            assert_eq!(sent.len(), 3);
            assert!(sent[1..].iter().all(|digest| digest != &sent[0]));
            let expected_warmup = body_digest(&serde_json::to_vec(&vec![&sent[0]]).unwrap());
            let mut measured = sent[1..].to_vec();
            measured.sort_unstable();
            let expected_measured = body_digest(&serde_json::to_vec(&measured).unwrap());
            assert_eq!(
                metadata["overlap_report"]["warmup_body_digests_sha256"],
                expected_warmup
            );
            assert_eq!(
                metadata["overlap_report"]["measured_body_digests_sha256"],
                expected_measured
            );
        }
    }
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}
