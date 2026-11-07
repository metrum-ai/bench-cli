// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
#![cfg(feature = "tokenizer")]

use axum::{body::Bytes, extract::State, routing::post, Json, Router};
use metrum_ai_bench::cache_workload::sha256;
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    process::Command,
    sync::{Arc, Mutex},
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn frozen_policy_wire_has_measured_overhead_and_stage_unique_ordinals_but_not_zero_header_sharing(
) {
    // Synthetic character BPE, not production-model or physical cache evidence.
    let bodies = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let app=Router::new().route("/chat/completions",post(|State(bodies):State<Arc<Mutex<Vec<Vec<u8>>>>>, bytes:Bytes| async move {
        bodies.lock().unwrap().push(bytes.to_vec());
        Json(json!({"choices":[{"message":{"role":"assistant","content":"ok"}}],"usage":{"prompt_tokens":23,"completion_tokens":1}}))
    })).with_state(bodies.clone());
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let vocab: serde_json::Map<String, Value> = (32_u8..127)
        .map(|c| ((c as char).to_string(), json!(c - 32)))
        .collect();
    let tokenizer=serde_json::to_vec(&json!({"version":"1.0","truncation":null,"padding":null,"added_tokens":[],"normalizer":null,"pre_tokenizer":null,"post_processor":null,"decoder":null,"model":{"type":"BPE","dropout":null,"unk_token":null,"continuing_subword_prefix":null,"end_of_word_suffix":null,"fuse_unk":false,"byte_fallback":false,"vocab":vocab,"merges":[]}})).unwrap();
    std::fs::write(dir.path().join("tokenizer.json"), &tokenizer).unwrap();
    for (name, content) in [("warm", "a"), ("measured", "b")] {
        std::fs::write(
            dir.path().join(format!("{name}.jsonl")),
            json!({"session_id":name,"messages":[{"role":"user","content":content}]}).to_string(),
        )
        .unwrap();
    }
    for policy in ["bust-request", "isolate-run"] {
        for long_header in [false, true] {
            bodies.lock().unwrap().clear();
            let template = format!(
                "{}{{% for message in messages %}}{{{{ message.content }}}}{{% endfor %}}",
                if long_header {
                    "H".repeat(64)
                } else {
                    String::new()
                }
            );
            std::fs::write(dir.path().join("chat.jinja"), &template).unwrap();
            let config = json!({"tokenizer_path":"tokenizer.json","template_path":"chat.jinja","identity":{"tokenizer_id":"fixture/ascii","tokenizer_revision":"a".repeat(40),"tokenizer_sha256":sha256(&tokenizer),"chat_template_sha256":sha256(template.as_bytes()),"add_special_tokens":false,"add_generation_prompt":true,"hint_nonce_included":false,"renderer_version":metrum_ai_bench::chat_rendering::RENDERER_VERSION},"special_tokens":{},"max_artifact_bytes":16384,"max_body_bytes":8192,"max_rendered_bytes":256,"max_tokens":256,"template_fuel":10000});
            std::fs::write(dir.path().join("render.json"), config.to_string()).unwrap();
            let csv = dir.path().join(format!("{policy}-{long_header}.csv"));
            let marker = dir.path().join(format!("{policy}.hook"));
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"));
            cmd.args([
                "--url",
                &format!("http://{address}/chat/completions"),
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
                "--session-request-roles",
                "user",
                "--warmup-block-size-tokens",
                "32",
                "--cache-policy",
                policy,
            ])
            .arg("--sessions")
            .arg(dir.path().join("measured.jsonl"))
            .arg("--warmup-sessions")
            .arg(dir.path().join("warm.jsonl"))
            .arg("--warmup-render-config")
            .arg(dir.path().join("render.json"))
            .arg("--csv")
            .arg(&csv)
            .arg("--html")
            .arg(dir.path().join(format!("{policy}-{long_header}.html")));
            if long_header {
                cmd.args(["--allow-unverified-warmup", "--stage-reset-scope", "prefix"])
                    .arg("--stage-hook")
                    .arg(format!("touch '{}'", marker.display()));
            }
            let out = cmd.output().unwrap();
            let sent = bodies.lock().unwrap().clone();
            if long_header {
                assert!(!out.status.success());
                assert!(sent.is_empty());
                assert!(!marker.exists());
                let err = String::from_utf8_lossy(&out.stderr);
                assert!(err.contains("residual_shared_full_blocks=Some(2)"), "{err}");
                continue;
            }
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(sent.len(), 8);
            let mut rows: Vec<metrum_ai_bench::strategic::BenchRecord> =
                csv::Reader::from_path(&csv)
                    .unwrap()
                    .deserialize()
                    .collect::<Result<_, _>>()
                    .unwrap();
            rows.sort_by_key(|r| r.seq);
            let mut recorded = Vec::new();
            let mut nonces = BTreeSet::new();
            let mut ordinals = BTreeSet::new();
            for row in rows {
                let hash = row.body_sha256.unwrap();
                recorded.push(hash);
                let e: Value =
                    serde_json::from_str(row.cache_policy_effects.as_deref().unwrap()).unwrap();
                assert_eq!(e["nonce_added_characters"], 22);
                assert_eq!(e["nonce_added_tokens"], 22);
                assert_eq!(e["rendered_body_tokens_before"], 1);
                assert_eq!(e["rendered_body_tokens_after"], 23);
                assert_eq!(e["token_scope"], "full_rendered_body_difference");
                assert_eq!(e["residual_shared_full_blocks"], 0);
                assert_eq!(e["residual_verdict"], "Pass");
                if policy == "bust-request" {
                    ordinals.insert(e["nonce_identity_sequence"].as_u64().unwrap());
                }
            }
            let mut observed: Vec<_> = sent.iter().map(|bytes| sha256(bytes)).collect();
            recorded.sort();
            observed.sort();
            assert_eq!(recorded, observed);
            for bytes in &sent {
                let body: Value = serde_json::from_slice(bytes).unwrap();
                let text = body["messages"][0]["content"].as_str().unwrap();
                assert_eq!(text.chars().count(), 23);
                assert_eq!(&text[20..22], "] ");
                nonces.insert(text[..22].to_owned());
            }
            if policy == "bust-request" {
                assert_eq!(nonces.len(), 8);
                assert_eq!(ordinals, (0..8).collect());
            } else {
                assert_eq!(nonces.len(), 1);
            }
            let summary: Value = serde_json::from_slice(&out.stdout).unwrap();
            let stages = summary["config"]["warmup_stages"].as_array().unwrap();
            assert_eq!(stages.len(), 2);
            for stage in stages {
                assert_eq!(stage["evidence"]["overlap_report"]["verdict"], "Pass");
                assert_eq!(
                    stage["evidence"]["overlap_report"]["max_shared_full_blocks"],
                    0
                );
                for proof in stage["evidence"]["rendering_evidence"].as_array().unwrap() {
                    assert!(observed.iter().any(|h| proof["body_sha256"] == *h));
                }
            }
        }
    }
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}
