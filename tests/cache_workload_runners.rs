// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use axum::{body::Bytes, extract::State, response::IntoResponse, routing::post, Json, Router};
use metrum_ai_bench::cache_workload::sha256;
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug)]
enum Runner {
    Llm,
    Strategic,
}

fn command(runner: Runner, url: &str, prompts: &Path, dir: &Path, streaming: bool) -> Command {
    let mut command = match runner {
        Runner::Llm => {
            let mut command = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-llm"));
            command
                .args([
                    "--scenario",
                    "cache-fixture",
                    "--mode",
                    "chat",
                    "--num-requests",
                    "4",
                    "--concurrency",
                    "1",
                ])
                .arg("--data-log")
                .arg(dir.join("requests.jsonl"))
                .arg("--debug-log")
                .arg(dir.join("debug.log"))
                .arg("--error-log")
                .arg(dir.join("errors.log"));
            command
        }
        Runner::Strategic => {
            let mut command = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"));
            command
                .args(["--sweep", "1", "--requests-per-stage", "4"])
                .arg("--csv")
                .arg(dir.join("requests.csv"))
                .arg("--html")
                .arg(dir.join("report.html"));
            command
        }
    };
    command
        .args([
            "--url",
            url,
            "--api-key",
            "fixture",
            "--model",
            "mock",
            "--max-tokens",
            "4",
            "--temperature",
            "0",
        ])
        .arg("--prompts")
        .arg(prompts);
    if streaming {
        command.arg("--streaming");
    }
    command
}

fn assert_evidence(runner: Runner, dir: &Path, received: &[Vec<u8>]) {
    let hashes: Vec<String> = received.iter().map(|bytes| sha256(bytes)).collect();
    let recorded = match runner {
        Runner::Llm => {
            let text = std::fs::read_to_string(dir.join("requests.jsonl")).unwrap();
            let mut rows: Vec<Value> = text
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .filter(|row| {
                    row["schema_version"]
                        .as_str()
                        .is_some_and(|version| version.contains("request.v"))
                })
                .collect();
            rows.sort_by_key(|row| row["seq"].as_u64().unwrap());
            rows.iter()
                .map(|row| {
                    assert_eq!(
                        row["modality_labels"]["cache_workload_version"],
                        "cache-workload.v1"
                    );
                    row["modality_labels"]["body_sha256"]
                        .as_str()
                        .unwrap()
                        .to_owned()
                })
                .collect::<Vec<_>>()
        }
        Runner::Strategic => {
            let mut rows: Vec<metrum_ai_bench::strategic::BenchRecord> =
                csv::Reader::from_path(dir.join("requests.csv"))
                    .unwrap()
                    .deserialize()
                    .collect::<Result<_, _>>()
                    .unwrap();
            rows.sort_by_key(|row| row.seq);
            rows.into_iter()
                .map(|row| {
                    assert_eq!(
                        row.cache_workload_version.as_deref(),
                        Some("cache-workload.v1")
                    );
                    row.body_sha256.unwrap()
                })
                .collect()
        }
    };
    assert_eq!(
        recorded, hashes,
        "{runner:?} hashes must bind exact received wire bytes"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cross_runner_order_replay_and_mutation_have_exact_wire_evidence() {
    let bodies = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let app = Router::new().route("/chat/completions", post(|State(bodies): State<Arc<Mutex<Vec<Vec<u8>>>>>, bytes: Bytes| async move {
        bodies.lock().unwrap().push(bytes.to_vec());
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        if body["stream"] == true {
            let token = json!({"choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":null}]});
            let done = json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"completion_tokens":1,"total_tokens":5}});
            ([("content-type", "text/event-stream")], format!("data: {token}\n\ndata: {done}\n\ndata: [DONE]\n\n")).into_response()
        } else {
            Json(json!({"choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],
                "usage":{"prompt_tokens":4,"completion_tokens":1,"total_tokens":5}})).into_response()
        }
    })).with_state(bodies.clone());
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let root = tempfile::tempdir().unwrap();
    let prompts = root.path().join("prompts.jsonl");
    let sequence = root.path().join("sequence.jsonl");
    let rows: Vec<String> = [(2, "gamma"), (0, "alpha")].iter().enumerate().map(|(seq, (index, text))| json!({
        "seq":seq,"prompt_index":index,"prompt_sha256_16":metrum_ai_bench::prompt_sequence::prompt_fingerprint(text),
        "prompt_bytes":text.len(),"cohort":0
    }).to_string()).collect();
    std::fs::write(&sequence, rows.join("\n")).unwrap();
    let url = format!("http://{address}/chat/completions");
    for streaming in [false, true] {
        std::fs::write(
            &prompts,
            "{\"prompt\":\"alpha\"}\n{\"prompt\":\"beta\"}\n{\"prompt\":\"gamma\"}\n",
        )
        .unwrap();
        for order in ["preserve", "seeded-shuffle", "prescribed"] {
            let mut baseline = None;
            for runner in [Runner::Llm, Runner::Strategic] {
                let dir = tempfile::tempdir().unwrap();
                bodies.lock().unwrap().clear();
                let mut command = command(runner, &url, &prompts, dir.path(), streaming);
                let seed = match runner {
                    Runner::Llm => "7",
                    Runner::Strategic => "999",
                };
                let order_seed = if order == "prescribed" && matches!(runner, Runner::Strategic) {
                    "314"
                } else {
                    "42"
                };
                command.args([
                    "--cache-policy",
                    "reuse",
                    "--prompt-order",
                    order,
                    "--seed",
                    seed,
                    "--order-seed",
                    order_seed,
                ]);
                let log = dir.path().join("effective.jsonl");
                command.arg("--prompt-sequence-log").arg(&log);
                if order == "prescribed" {
                    command.arg("--prompt-sequence").arg(&sequence);
                }
                let output = command.output().unwrap();
                assert!(
                    output.status.success(),
                    "{runner:?} {order}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let received = bodies.lock().unwrap().clone();
                assert_eq!(received.len(), 4);
                assert_evidence(runner, dir.path(), &received);
                if let Some(expected) = &baseline {
                    assert_eq!(&received, expected, "cross-runner {order} parity");
                } else {
                    baseline = Some(received.clone());
                }
                let log_rows: Vec<Value> = std::fs::read_to_string(log)
                    .unwrap()
                    .lines()
                    .map(|line| serde_json::from_str(line).unwrap())
                    .collect();
                for (logged, bytes) in log_rows.iter().zip(&received) {
                    let body: Value = serde_json::from_slice(bytes).unwrap();
                    let prompt = body["messages"][0]["content"].as_str().unwrap();
                    assert_eq!(
                        logged["prompt_sha256_16"],
                        metrum_ai_bench::prompt_sequence::prompt_fingerprint(prompt)
                    );
                }
            }
        }
        // One repeated body distinguishes request-scoped busting from run-scoped isolation.
        std::fs::write(&prompts, "{\"prompt\":\"repeat\"}\n").unwrap();
        for runner in [Runner::Llm, Runner::Strategic] {
            for policy in ["bust-request", "isolate-run"] {
                let mut previous_run = None;
                for _ in 0..2 {
                    let dir = tempfile::tempdir().unwrap();
                    bodies.lock().unwrap().clear();
                    let output = command(runner, &url, &prompts, dir.path(), streaming)
                        .args(["--cache-policy", policy, "--prompt-order", "preserve"])
                        .output()
                        .unwrap();
                    assert!(
                        output.status.success(),
                        "{}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    let received = bodies.lock().unwrap().clone();
                    assert_evidence(runner, dir.path(), &received);
                    let distinct: std::collections::BTreeSet<_> =
                        received.iter().map(|body| sha256(body)).collect();
                    assert_eq!(distinct.len(), if policy == "bust-request" { 4 } else { 1 });
                    if let Some(previous) = &previous_run {
                        assert_ne!(&received, previous);
                    }
                    previous_run = Some(received);
                }
            }
            bodies.lock().unwrap().clear();
            let dir = tempfile::tempdir().unwrap();
            let output = command(runner, &url, &prompts, dir.path(), streaming)
                .args(["--cache-policy", "reuse"])
                .arg("--prompt-sequence")
                .arg(&sequence)
                .output()
                .unwrap();
            assert!(!output.status.success(), "mismatched replay must fail");
            assert!(
                bodies.lock().unwrap().is_empty(),
                "replay mismatch sent traffic"
            );
        }
    }
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}
