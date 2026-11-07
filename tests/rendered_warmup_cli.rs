// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
#![cfg(feature = "tokenizer")]

use axum::{body::Bytes, extract::State, routing::post, Json, Router};
use metrum_ai_bench::warmup_isolation::body_digest;
use serde_json::{json, Value};
use std::process::Command;
use std::sync::{Arc, Mutex};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pinned_renderer_binds_actual_wire_and_rejects_overlap_or_unrenderable_before_hooks() {
    let bodies = Arc::new(Mutex::new(Vec::<String>::new()));
    let app = Router::new()
        .route(
            "/chat/completions",
            post(
                |State(bodies): State<Arc<Mutex<Vec<String>>>>, bytes: Bytes| async move {
                    bodies.lock().unwrap().push(body_digest(&bytes));
                    Json(
                        json!({"choices":[{"message":{"role":"assistant","content":"ok"}}],
                "usage":{"prompt_tokens":1,"completion_tokens":1}}),
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
    let tokenizer = br#"{"version":"1.0","truncation":null,"padding":null,"added_tokens":[],"normalizer":null,"pre_tokenizer":null,"post_processor":null,"decoder":null,"model":{"type":"BPE","dropout":null,"unk_token":null,"continuing_subword_prefix":null,"end_of_word_suffix":null,"fuse_unk":false,"byte_fallback":false,"vocab":{"a":0,"b":1},"merges":[]}}"#;
    let template = b"{% for message in messages %}{{ message.content }}{% endfor %}";
    std::fs::write(dir.path().join("tokenizer.json"), tokenizer).unwrap();
    std::fs::write(dir.path().join("chat.jinja"), template).unwrap();
    let config = dir.path().join("render.json");
    std::fs::write(&config,json!({
        "tokenizer_path":"tokenizer.json","template_path":"chat.jinja",
        "identity":{"tokenizer_id":"fixture/bpe","tokenizer_revision":"a".repeat(40),
          "tokenizer_sha256":body_digest(tokenizer),"chat_template_sha256":body_digest(template),
          "add_special_tokens":false,"add_generation_prompt":true,"hint_nonce_included":false,
          "renderer_version":metrum_ai_bench::chat_rendering::RENDERER_VERSION},
        "special_tokens":{},"max_artifact_bytes":8192,"max_body_bytes":8192,
        "max_rendered_bytes":128,"max_tokens":128,"template_fuel":10000
    }).to_string()).unwrap();
    let measured = dir.path().join("measured.jsonl");
    let warmup = dir.path().join("warmup.jsonl");
    std::fs::write(
        &warmup,
        json!({"session_id":"warm","messages":[{"role":"user","content":"a"}]}).to_string(),
    )
    .unwrap();
    for (name, content, pass) in [
        ("pass", "b", true),
        ("overlap", "a", false),
        ("unrenderable", "z", false),
    ] {
        bodies.lock().unwrap().clear();
        std::fs::write(
            &measured,
            json!({"session_id":"measure","messages":[{"role":"user","content":content}]})
                .to_string(),
        )
        .unwrap();
        let marker = dir.path().join(format!("{name}.hook"));
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"));
        cmd.args([
            "--url",
            &format!("http://{address}/chat/completions"),
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
            "--warmup-block-size-tokens",
            "1",
        ])
        .arg("--sessions")
        .arg(&measured)
        .arg("--warmup-sessions")
        .arg(&warmup)
        .arg("--warmup-render-config")
        .arg(&config)
        .arg("--html")
        .arg(dir.path().join(format!("{name}.html")));
        if !pass {
            cmd.arg("--allow-unverified-warmup")
                .arg("--stage-hook")
                .arg(format!("touch '{}'", marker.display()))
                .args(["--stage-reset-scope", "prefix"]);
        }
        let out = cmd.output().unwrap();
        assert_eq!(
            out.status.success(),
            pass,
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let sent = bodies.lock().unwrap();
        if !pass {
            assert!(sent.is_empty());
            assert!(!marker.exists());
            continue;
        }
        assert_eq!(sent.len(), 3);
        let summary: Value = serde_json::from_slice(&out.stdout).unwrap();
        let metadata = &summary["config"]["warmup"];
        assert_eq!(metadata["overlap_report"]["verdict"], "Pass");
        assert_eq!(metadata["overlap_report"]["pairs_over_threshold_total"], 0);
        assert_eq!(metadata["rendered_tokens_available"], true);
        assert_eq!(
            metadata["overlap_report"]["warmup_body_digests_sha256"],
            body_digest(&serde_json::to_vec(&vec![&sent[0]]).unwrap())
        );
        let mut measured = sent[1..].to_vec();
        measured.sort_unstable();
        assert_eq!(
            metadata["overlap_report"]["measured_body_digests_sha256"],
            body_digest(&serde_json::to_vec(&measured).unwrap())
        );
        let evidence = metadata["rendering_evidence"].as_array().unwrap();
        assert_eq!(evidence.len(), 3);
        for (row, hash) in evidence.iter().zip(sent.iter()) {
            assert_eq!(row["body_sha256"], *hash);
            assert!(row.get("token_ids").is_none());
            assert_eq!(row["server_tokenize_cross_check"], "NotRequested");
        }
    }
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}
