// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use axum::{
    body::{Body, Bytes},
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use metrum_ai_bench::{
    cache_cohorts::{summarize_cohorts, CohortSample},
    cached_tokens::{CachedTokenEvidence, HitCohort},
    measurement::TtftSource,
    record::Phase,
};
use serde_json::{json, Value};
use std::{
    process::Command,
    sync::{Arc, Mutex},
};

type Seen = Arc<Mutex<Vec<usize>>>;

fn usage(case: usize) -> Value {
    let mut usage = json!({"prompt_tokens":8,"completion_tokens":2,"total_tokens":10});
    let cached = match case {
        0 => return usage,
        1 => Value::Null,
        2 => json!(0),
        3 => json!(3),
        4 | 7 => json!(8),
        5 => json!(9),
        6 => json!(-1),
        _ => unreachable!(),
    };
    usage["prompt_tokens_details"] = json!({"cached_tokens":cached});
    usage
}

async fn respond(State(seen): State<Seen>, bytes: Bytes) -> axum::response::Response {
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    let case: usize = body["messages"][0]["content"]
        .as_str()
        .unwrap()
        .strip_prefix("case-")
        .unwrap()
        .parse()
        .unwrap();
    seen.lock().unwrap().push(case);
    if body["stream"] == true {
        let mut frames = vec![
            format!(
                "data: {}\n\n",
                json!({"choices":[{"index":0,"delta":{"reasoning_content":"thought"}}]})
            ),
            format!(
                "data: {}\n\n",
                json!({"choices":[{"index":0,"delta":{"content":"answer"},"finish_reason":"stop"}]})
            ),
            format!("data: {}\n\n", json!({"choices":[],"usage":usage(case)})),
        ];
        frames.push(if case == 7 {
            format!(
                "data: {}\n\n",
                json!({"error":{"message":"fixture stream error"}})
            )
        } else {
            "data: [DONE]\n\n".into()
        });
        let stream = futures_util::stream::unfold(frames.into_iter(), |mut frames| async move {
            let frame = frames.next()?;
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            Some((
                Ok::<_, std::convert::Infallible>(Bytes::from(frame)),
                frames,
            ))
        });
        (
            [("content-type", "text/event-stream")],
            Body::from_stream(stream),
        )
            .into_response()
    } else if case == 7 {
        // An HTTP error body is not a successful usage report; do not attribute it.
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":{"message":"fixture error"},"usage":usage(case)})),
        )
            .into_response()
    } else {
        Json(json!({"choices":[{"message":{"role":"assistant","content":"answer"},"finish_reason":"stop"}],"usage":usage(case)})).into_response()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unary_and_sse_evidence_survives_both_runners_and_reconciles_cohorts() {
    let seen: Seen = Arc::default();
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/chat/completions", post(respond))
        .with_state(seen.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    for strategic in [false, true] {
        for streaming in [false, true] {
            seen.lock().unwrap().clear();
            let dir = tempfile::tempdir().unwrap();
            let prompts = dir.path().join("prompts.jsonl");
            std::fs::write(
                &prompts,
                (0..8)
                    .map(|case| json!({"prompt":format!("case-{case}")}).to_string() + "\n")
                    .collect::<String>(),
            )
            .unwrap();
            let mut command = Command::new(if strategic {
                env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic")
            } else {
                env!("CARGO_BIN_EXE_metrum-ai-bench-cli-llm")
            });
            command
                .args([
                    "--url",
                    &format!("http://{address}/chat/completions"),
                    "--api-key",
                    "dummy",
                    "--model",
                    "mock",
                    "--max-tokens",
                    "2",
                    "--cache-policy",
                    "reuse",
                    "--prompt-order",
                    "preserve",
                    "--warmup-requests",
                    "2",
                ])
                .arg("--prompts")
                .arg(&prompts);
            if streaming {
                command.arg("--streaming");
            } else {
                command.arg("--infer-ttft-from-first-byte");
            }
            if strategic {
                command
                    .args(["--sweep", "1", "--requests-per-stage", "8"])
                    .arg("--csv")
                    .arg(dir.path().join("requests.csv"))
                    .arg("--html")
                    .arg(dir.path().join("report.html"));
            } else {
                command
                    .args([
                        "--scenario",
                        "cached-usage",
                        "--mode",
                        "chat",
                        "--num-requests",
                        "10",
                        "--concurrency",
                        "1",
                    ])
                    .arg("--data-log")
                    .arg(dir.path().join("requests.jsonl"))
                    .arg("--debug-log")
                    .arg(dir.path().join("debug.log"))
                    .arg("--error-log")
                    .arg(dir.path().join("error.log"));
            }
            let output = command.output().unwrap();
            // Exit-policy coverage is separate; this fixture deliberately has one
            // failed request. Both normal and request-failure exits are permitted,
            // but panics, signals, configuration errors and missing rows are not.
            assert!(
                matches!(output.status.code(), Some(0 | 1)),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let mut rows: Vec<Value> = if strategic {
                let mut reader = csv::Reader::from_path(dir.path().join("requests.csv")).unwrap();
                let headers = reader.headers().unwrap().clone();
                let first_any_column = headers
                    .iter()
                    .position(|header| header == "ttft_first_any_s")
                    .unwrap();
                reader
                    .records()
                    .map(|row| {
                        let row = row.unwrap();
                        let typed: metrum_ai_bench::strategic::BenchRecord =
                            row.deserialize(Some(&headers)).unwrap();
                        let raw = row.get(first_any_column).unwrap();
                        let first_any = if raw.is_empty() {
                            None
                        } else {
                            Some(raw.parse::<f64>().unwrap())
                        };
                        assert_eq!(
                            first_any,
                            typed.first_any_s(),
                            "CSV-derived first-any must match the typed accessor"
                        );
                        let mut value = serde_json::to_value(typed).unwrap();
                        value["ttft_first_any_s"] = json!(first_any);
                        value
                    })
                    .collect()
            } else {
                std::fs::read_to_string(dir.path().join("requests.jsonl"))
                    .unwrap()
                    .lines()
                    .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                    .filter(|row| {
                        row["schema_version"]
                            .as_str()
                            .is_some_and(|version| version.contains("request.v"))
                    })
                    .collect()
            };
            rows.sort_by_key(|row| row["seq"].as_u64().unwrap());
            let cases = seen.lock().unwrap().clone();
            assert_eq!(rows.len(), 10);
            assert_eq!(cases.len(), rows.len());
            let evidence: Vec<CachedTokenEvidence> = rows
                .iter()
                .zip(&cases)
                .map(|(row, &case)| {
                    let actual: CachedTokenEvidence =
                        serde_json::from_value(row["cache_evidence"].clone()).unwrap();
                    let expected = if case == 7 && !streaming {
                        CachedTokenEvidence::default()
                    } else {
                        CachedTokenEvidence::from_usage(&usage(case))
                    };
                    assert_eq!(
                        actual, expected,
                        "strategic={strategic} streaming={streaming} case={case}"
                    );
                    if case == 7 {
                        assert!(row.get("error").is_some_and(|error| !error.is_null()));
                    } else if streaming {
                        assert!(
                            row["ttft_first_any_s"].as_f64().unwrap()
                                <= row["ttft_visible_answer_s"].as_f64().unwrap()
                        );
                    } else {
                        assert_eq!(row["ttft_source"], "first_byte_approx");
                        assert!(row["ttft_visible_answer_s"].is_null());
                    }
                    actual
                })
                .collect();
            let summary =
                summarize_cohorts(
                    rows.iter()
                        .zip(&evidence)
                        .map(|(row, evidence)| CohortSample {
                            phase: if row["warmup"] == true || row["phase"] == "warmup" {
                                Phase::Warmup
                            } else {
                                Phase::Measure
                            },
                            success: !row.get("error").is_some_and(|error| !error.is_null()),
                            valid: row["valid"].as_bool(),
                            evidence,
                            headline_ttft_s: row["ttft_first_any_s"]
                                .as_f64()
                                .or(row["ttft_s"].as_f64()),
                            headline_source: match row["ttft_source"].as_str() {
                                Some("stream") => Some(TtftSource::Stream),
                                Some("first_byte_approx") => Some(TtftSource::FirstByteApprox),
                                Some("first_token_fallback") => {
                                    Some(TtftSource::FirstTokenFallback)
                                }
                                _ => None,
                            },
                            visible_answer_ttft_s: row["ttft_visible_answer_s"].as_f64(),
                            turn: None,
                            prefix_group: None,
                        }),
                );
            if strategic {
                let report: Value = serde_json::from_slice(&output.stdout).unwrap();
                let expected = serde_json::to_value(&summary).unwrap();
                assert_eq!(
                    report["cache_cohorts"]["schema_version"],
                    "cached-token-cohort-report.v1"
                );
                assert_eq!(report["cache_cohorts"]["all_stages"], expected);
                assert_eq!(
                    report["cache_cohorts"]["stages"].as_array().unwrap().len(),
                    1
                );
                assert_eq!(report["cache_cohorts"]["stages"][0]["cohorts"], expected);
            } else {
                assert!(String::from_utf8_lossy(&output.stderr)
                    .contains("cold=1 partial=1 full=1 unknown=5 measured=8"));
            }
            assert_eq!(summary.measured_requests, 8);
            assert_eq!(summary.warmup_excluded, 2);
            assert_eq!(summary.errors_excluded_from_timing, 1);
            assert_eq!(summary.counts[&HitCohort::Cold], 1);
            assert_eq!(summary.counts[&HitCohort::Partial], 1);
            assert_eq!(summary.counts[&HitCohort::Full], 1);
            assert_eq!(summary.counts[&HitCohort::Unknown], 5);
            assert_eq!(
                summary
                    .groups
                    .iter()
                    .map(|group| group.timing_eligible_requests)
                    .sum::<usize>(),
                7
            );
        }
    }
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}
