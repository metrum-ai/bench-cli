// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use axum::{routing::post, Router};
use std::process::Command;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unavailable_canonical_loader_never_falls_back_or_runs_reset_hooks() {
    let requests = Arc::new(AtomicUsize::new(0));
    let counter = requests.clone();
    let app = Router::new().route(
        "/chat/completions",
        post(move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                "unexpected traffic"
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("input.jsonl");
    // Intentionally valid as legacy input, but canonical mode must never use it.
    std::fs::write(&source, "{\"prompt\":\"not a canonical record\"}\n").unwrap();
    let marker = dir.path().join("hook-must-not-run");
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"))
        .args([
            "--url",
            &format!("http://{address}/chat/completions"),
            "--model",
            "fixture",
            "--api-key",
            "dummy",
            "--sweep",
            "1",
            "--requests-per-stage",
            "1",
            "--max-tokens",
            "8",
        ])
        .arg("--canonical-jsonl")
        .arg(&source)
        .arg("--stage-hook")
        .arg(format!("touch {}", marker.display()))
        .args(["--stage-reset-scope", "cpu", "--pre-warmup-reset"])
        .arg("--csv")
        .arg(dir.path().join("requests.csv"))
        .arg("--html")
        .arg(dir.path().join("report.html"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("canonical input loader is not integrated"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    assert!(
        !marker.exists(),
        "reset ran before canonical input validation"
    );
    assert!(!dir.path().join("requests.csv").exists());
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}
