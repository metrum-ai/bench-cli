// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Deterministic OpenAI-compatible mock server for local and CI benchmarks.

use anyhow::Result;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Deterministic mock inference server for local and CI benchmarks"
)]
struct Args {
    #[arg(long, default_value = "127.0.0.1:8080")]
    listen: String,
    #[arg(long, default_value_t = 0)]
    latency_ms: u64,
    #[arg(long, default_value_t = 0)]
    fail_every: usize,
}

struct AppState {
    latency: Duration,
    fail_every: usize,
    requests: AtomicUsize,
    running: AtomicUsize,
    preemptions: AtomicU64,
}

struct RunningGuard(Arc<AppState>);

impl Drop for RunningGuard {
    fn drop(&mut self) {
        self.0.running.fetch_sub(1, Ordering::Relaxed);
    }
}

async fn infer(
    State(state): State<Arc<AppState>>,
    Json(request): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let sequence = state.requests.fetch_add(1, Ordering::Relaxed) + 1;
    state.running.fetch_add(1, Ordering::Relaxed);
    let _guard = RunningGuard(state.clone());
    if !state.latency.is_zero() {
        tokio::time::sleep(state.latency).await;
    }
    if state.fail_every > 0 && sequence % state.fail_every == 0 {
        state.preemptions.fetch_add(1, Ordering::Relaxed);
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let model = request
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("metrumbench-mock");
    if request.get("input").is_some() {
        return Ok(Json(json!({
            "object":"list",
            "model":model,
            "data":[{"object":"embedding","index":0,"embedding":[0.0,0.5,1.0]}],
            "usage":{"prompt_tokens":3,"total_tokens":3}
        })));
    }
    if request.get("documents").is_some() {
        return Ok(Json(json!({
            "model":model,
            "results":[{"index":0,"relevance_score":0.95}],
            "usage":{"total_tokens":8}
        })));
    }
    let message = if request.get("tools").is_some() {
        let arguments = mock_schema_value(
            request
                .pointer("/tools/0/function/parameters")
                .unwrap_or(&json!({"type":"object"})),
        );
        json!({
            "role":"assistant","content":null,
            "tool_calls":[{"id":"call_mock","type":"function","function":{"name":request.pointer("/tools/0/function/name").and_then(Value::as_str).unwrap_or("tool"),"arguments":arguments.to_string()}}]
        })
    } else if request.pointer("/response_format/type") == Some(&json!("json_schema")) {
        let schema = request
            .pointer("/response_format/json_schema/schema")
            .unwrap_or(&Value::Null);
        json!({"role":"assistant","content":mock_schema_value(schema).to_string()})
    } else {
        json!({"role":"assistant","content":"Hello from metrumbench."})
    };
    Ok(Json(json!({
        "id":format!("chatcmpl-{sequence}"),"object":"chat.completion","model":model,
        "choices":[{"index":0,"message":message,"finish_reason":"stop"}],
        "usage":{"prompt_tokens":8,"completion_tokens":4,"total_tokens":12}
    })))
}

fn mock_schema_value(schema: &Value) -> Value {
    match schema.get("type").and_then(Value::as_str) {
        Some("object") => {
            let required = schema
                .get("required")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let properties = schema.get("properties").and_then(Value::as_object);
            Value::Object(
                required
                    .into_iter()
                    .filter_map(|key| {
                        let key = key.as_str()?;
                        let property = properties
                            .and_then(|properties| properties.get(key))
                            .unwrap_or(&Value::Null);
                        Some((key.to_string(), mock_schema_value(property)))
                    })
                    .collect(),
            )
        }
        Some("array") => Value::Array(vec![mock_schema_value(
            schema.get("items").unwrap_or(&Value::Null),
        )]),
        Some("string") => json!("mock"),
        Some("integer") => json!(1),
        Some("number") => json!(1.0),
        Some("boolean") => json!(true),
        _ => Value::Null,
    }
}

async fn metrics(State(state): State<Arc<AppState>>) -> String {
    format!(
        "# TYPE vllm:num_requests_running gauge\nvllm:num_requests_running {}\n\
         # TYPE vllm:num_requests_waiting gauge\nvllm:num_requests_waiting 0\n\
         # TYPE vllm:gpu_cache_usage_perc gauge\nvllm:gpu_cache_usage_perc 0.25\n\
         # TYPE vllm:num_preemptions_total counter\nvllm:num_preemptions_total {}\n",
        state.running.load(Ordering::Relaxed),
        state.preemptions.load(Ordering::Relaxed)
    )
}

async fn models() -> Json<Value> {
    Json(json!({
        "object":"list",
        "data":[{"id":"metrumbench-mock","object":"model","owned_by":"metrum-ai"}]
    }))
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let state = Arc::new(AppState {
        latency: Duration::from_millis(args.latency_ms),
        fail_every: args.fail_every,
        requests: AtomicUsize::new(0),
        running: AtomicUsize::new(0),
        preemptions: AtomicU64::new(0),
    });
    let app = Router::new()
        .route("/health", get(|| async { StatusCode::OK }))
        .route("/healthz", get(|| async { StatusCode::OK }))
        .route("/ready", get(|| async { StatusCode::OK }))
        .route("/metrics", get(metrics))
        .route("/v1/models", get(models))
        .route("/v1/chat/completions", post(infer))
        .route("/v1/completions", post(infer))
        .route("/v1/embeddings", post(infer))
        .route("/v1/rerank", post(infer))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(&args.listen).await?;
    println!("metrumbench-mock-server listening on {}", args.listen);
    axum::serve(listener, app).await?;
    Ok(())
}
