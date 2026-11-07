// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::io::Write;
use std::process::Command;

use axum::{extract::State, routing::post, Json, Router};
use serde_json::{json, Value};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reset_is_after_warmup_before_measurement_once_per_stage_and_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let events = dir.path().join("events.txt");
    let app = Router::new()
        .route(
            "/chat/completions",
            post(|State(path): State<std::path::PathBuf>| async move {
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .unwrap();
                writeln!(file, "send").unwrap();
                Json(
                    json!({"choices":[{"message":{"role":"assistant","content":"ok"}}],
            "usage":{"prompt_tokens": 4,"completion_tokens": 1}}),
                )
            }),
        )
        .with_state(events.clone());
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let url = format!("http://{address}/chat/completions");
    let ack = r#"{"success":true,"scope":["resident","external"],"ready":true,"settled":true}"#;
    for pre_warmup in [false, true] {
        std::fs::write(&events, "").unwrap();
        let hook = format!(
            "printf 'reset\\n' >> '{}'; printf '%s' '{ack}'",
            events.display()
        );
        let mut command = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"));
        command
            .args([
                "--url",
                &url,
                "--model",
                "mock",
                "--sweep",
                "1,2",
                "--requests-per-stage",
                "2",
                "--warmup-requests",
                "2",
                "--max-tokens",
                "4",
                "--stage-hook",
                &hook,
                "--stage-reset-scope",
                "resident,external",
                "--stage-settle-ms",
                "1",
            ])
            .arg("--html")
            .arg(dir.path().join("report.html"));
        if pre_warmup {
            command.arg("--pre-warmup-reset");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let phase = if pre_warmup {
            "reset\nsend\nsend\nreset\nsend\nsend\n"
        } else {
            "send\nsend\nreset\nsend\nsend\n"
        };
        assert_eq!(std::fs::read_to_string(&events).unwrap(), phase.repeat(2));
        let summary: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            summary["config"]["stage_resets"].as_array().unwrap().len(),
            2
        );
        for stage in summary["config"]["stage_resets"].as_array().unwrap() {
            let resets = stage["resets"].as_array().unwrap();
            assert_eq!(resets.len(), if pre_warmup { 2 } else { 1 });
            assert_eq!(resets.last().unwrap()["position"], "after_warmup");
            let ack = &resets.last().unwrap()["acknowledgement"];
            assert_eq!(ack["acknowledged"], true);
            assert!(
                ack["settled_unix_ns"].as_u64().unwrap() >= ack["ready_unix_ns"].as_u64().unwrap()
            );
        }
        assert!(!String::from_utf8_lossy(&output.stdout).contains(&hook));
    }
    std::fs::write(&events, "").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"))
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
            "--stage-hook",
            "exit 7",
            "--stage-reset-scope",
            "resident",
        ])
        .arg("--html")
        .arg(dir.path().join("failed.html"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(std::fs::read_to_string(events).unwrap(), "send\n");
    assert!(String::from_utf8_lossy(&output.stderr).contains("stage reset"));
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}
