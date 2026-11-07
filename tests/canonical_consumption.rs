// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use axum::Router;
use std::process::Command;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_canonical_schema_never_falls_back_or_runs_reset_hooks() {
    let requests = Arc::new(AtomicUsize::new(0));
    let counter = requests.clone();
    let app = Router::new().fallback(move || {
        let counter = counter.clone();
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            "unexpected traffic"
        }
    });
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
            .contains("invalid canonical schema at source row 1"),
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_canonical_file_retains_wire_target_lineage_and_declared_controls() {
    use axum::{extract::State, routing::post, Json};
    use serde_json::{json, Value};
    type Seen = Arc<std::sync::Mutex<Vec<Value>>>;
    let seen: Seen = Arc::default();
    let app = Router::new().route("/chat/completions", post(|State(seen): State<Seen>, Json(body): Json<Value>| async move {
        seen.lock().unwrap().push(body);
        Json(json!({"choices":[{"message":{"role":"assistant","content":"ok"}}],"usage":{"prompt_tokens":4,"completion_tokens":1}}))
    })).with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("canonical.jsonl");
    let messages = json!([{"role":"system","content":"fixed"},{"role":"user","content":"first"},
        {"role":"assistant","content":"recorded answer"},{"role":"user","content":"next"}]);
    let source = json!({"source_id":"synthetic:fixture", "resolved_revision":null,"config":null,"split":null,
        "files":[],"adapter_id":"fixture","adapter_version":"1","license_spdx":"Apache-2.0","validation_manifest":null});
    let record = json!({"record_id":"target-original", "source_row_id":"row-original-7", "session_id":"session-original",
        "turn_index":u32::MAX, "group_id":"group-original", "trace_id":"trace-original", "arrival_offset_ms":73,
        "body":{"Chat":messages.clone()}, "expected_output_tokens":3, "source":source.clone()});
    std::fs::write(&path, record.to_string() + "\n").unwrap();
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
            "--session-mode",
            "closed-loop",
        ])
        .arg("--canonical-jsonl")
        .arg(&path)
        .arg("--csv")
        .arg(dir.path().join("requests.csv"))
        .arg("--html")
        .arg(dir.path().join("report.html"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["config"]["datasets"], json!([source]));
    assert_eq!(
        report["config"]["canonical_input"]["session_mode"],
        "closed-loop"
    );
    assert_eq!(
        report["config"]["canonical_input"]["session_scope"],
        "session_id_global_within_input"
    );
    assert_eq!(
        report["config"]["canonical_input"]["arrival_offsets"],
        "metadata_only_not_trace_replay"
    );
    {
        let seen = seen.lock().unwrap();
        assert_eq!(
            seen.len(),
            1,
            "a selected target must not be expanded into four prefixes"
        );
        assert_eq!(seen[0]["messages"], messages);
        assert_eq!(
            seen[0]["max_tokens"], 8,
            "source target is not a silent output cap"
        );
    }
    let rows: Vec<metrum_ai_bench::strategic::BenchRecord> =
        csv::Reader::from_path(dir.path().join("requests.csv"))
            .unwrap()
            .deserialize()
            .collect::<Result<_, _>>()
            .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].session_id.as_deref(), Some("session-original"));
    assert!(rows[0].turn.is_none());
    let lineage: Value =
        serde_json::from_str(rows[0].canonical_lineage_json.as_deref().unwrap()).unwrap();
    for key in [
        "record_id",
        "source_row_id",
        "session_id",
        "turn_index",
        "group_id",
        "trace_id",
        "arrival_offset_ms",
        "expected_output_tokens",
        "source",
    ] {
        assert_eq!(lineage[key], record[key], "lineage field {key}");
    }
    assert!(lineage.get("body").is_none());
    assert!(!lineage.to_string().contains("recorded answer"));
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}
