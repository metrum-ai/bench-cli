// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Deterministic OpenAI-compatible mock server for local and CI benchmarks.

use anyhow::Result;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
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
    #[arg(
        long,
        default_value_t = false,
        help = "Serve canned DCGM and all-smi Prometheus fixtures on /metrics and /metric"
    )]
    telemetry_fixture: bool,
}

struct AppState {
    latency: Duration,
    fail_every: usize,
    requests: AtomicUsize,
    running: AtomicUsize,
    preemptions: AtomicU64,
    telemetry_fixture: bool,
    scrapes: AtomicU64,
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
) -> Result<Response, StatusCode> {
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
        .unwrap_or("metrum-ai-bench-cli-mock");
    if request.get("input").is_some() {
        return Ok(Json(json!({
            "object":"list",
            "model":model,
            "data":[{"object":"embedding","index":0,"embedding":[0.0,0.5,1.0]}],
            "usage":{"prompt_tokens":3,"total_tokens":3}
        }))
        .into_response());
    }
    if request.get("documents").is_some() {
        return Ok(Json(json!({
            "model":model,
            "results":[{"index":0,"relevance_score":0.95}],
            "usage":{"total_tokens":8}
        }))
        .into_response());
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
        json!({"role":"assistant","content":"Hello from metrum-ai-bench-cli."})
    };
    let stream = request
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if stream {
        let content = message
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("ok");
        let id = format!("chatcmpl-{sequence}");
        let mut body = String::new();
        for (index, ch) in content.chars().enumerate() {
            let chunk = json!({
                "id": id,
                "object": "chat.completion.chunk",
                "model": model,
                "choices": [{
                    "index": 0,
                    "delta": {"content": ch.to_string()},
                    "finish_reason": null
                }]
            });
            body.push_str(&format!("data: {chunk}\n\n"));
            if index == 0 {
                // Keep chunks tiny so TTFT is meaningful under latency_ms.
            }
        }
        let final_chunk = json!({
            "id": id,
            "object": "chat.completion.chunk",
            "model": model,
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 8, "completion_tokens": 4, "total_tokens": 12}
        });
        body.push_str(&format!("data: {final_chunk}\n\n"));
        body.push_str("data: [DONE]\n\n");
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .body(body.into())
            .expect("sse response"));
    }
    Ok(Json(json!({
        "id":format!("chatcmpl-{sequence}"),"object":"chat.completion","model":model,
        "choices":[{"index":0,"message":message,"finish_reason":"stop"}],
        "usage":{"prompt_tokens":8,"completion_tokens":4,"total_tokens":12}
    }))
    .into_response())
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
    let scrape = state.scrapes.fetch_add(1, Ordering::Relaxed);
    if state.telemetry_fixture {
        return telemetry_fixture_body(scrape, &state);
    }
    format!(
        "# TYPE vllm:num_requests_running gauge\nvllm:num_requests_running {}\n\
         # TYPE vllm:num_requests_waiting gauge\nvllm:num_requests_waiting 0\n\
         # TYPE vllm:gpu_cache_usage_perc gauge\nvllm:gpu_cache_usage_perc 0.25\n\
         # TYPE vllm:num_preemptions_total counter\nvllm:num_preemptions_total {}\n",
        state.running.load(Ordering::Relaxed),
        state.preemptions.load(Ordering::Relaxed)
    )
}

fn telemetry_fixture_body(scrape: u64, state: &AppState) -> String {
    let power = 200.0 + (scrape % 50) as f64;
    let energy_mj = 1_000_000.0 + scrape as f64 * 250.0;
    let util = 40.0 + (scrape % 40) as f64;
    let running = state.running.load(Ordering::Relaxed);
    format!(
        "# HELP DCGM_FI_DEV_POWER_USAGE Power draw\n\
         # TYPE DCGM_FI_DEV_POWER_USAGE gauge\n\
         DCGM_FI_DEV_POWER_USAGE{{gpu=\"0\",UUID=\"GPU-mock\"}} {power}\n\
         # TYPE DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION counter\n\
         DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION{{gpu=\"0\",UUID=\"GPU-mock\"}} {energy_mj}\n\
         # TYPE DCGM_FI_DEV_GPU_UTIL gauge\n\
         DCGM_FI_DEV_GPU_UTIL{{gpu=\"0\",UUID=\"GPU-mock\"}} {util}\n\
         # TYPE DCGM_FI_DEV_FB_USED gauge\n\
         DCGM_FI_DEV_FB_USED{{gpu=\"0\",UUID=\"GPU-mock\"}} 12288\n\
         # TYPE DCGM_FI_DEV_SM_CLOCK gauge\n\
         DCGM_FI_DEV_SM_CLOCK{{gpu=\"0\",UUID=\"GPU-mock\"}} 1410\n\
         # TYPE DCGM_FI_DEV_GPU_TEMP gauge\n\
         DCGM_FI_DEV_GPU_TEMP{{gpu=\"0\",UUID=\"GPU-mock\"}} 62\n\
         # TYPE DCGM_FI_PROF_SM_ACTIVE gauge\n\
         DCGM_FI_PROF_SM_ACTIVE{{gpu=\"0\",UUID=\"GPU-mock\"}} 0.55\n\
         # TYPE all_smi_gpu_power_consumption_watts gauge\n\
         all_smi_gpu_power_consumption_watts{{gpu_id=\"0\",gpu_name=\"H100\"}} {power}\n\
         # TYPE all_smi_gpu_utilization gauge\n\
         all_smi_gpu_utilization{{gpu_id=\"0\",gpu_name=\"H100\"}} {util}\n\
         # TYPE all_smi_gpu_memory_used_bytes gauge\n\
         all_smi_gpu_memory_used_bytes{{gpu_id=\"0\",gpu_name=\"H100\"}} 12884901888\n\
         # TYPE all_smi_gpu_temperature_celsius gauge\n\
         all_smi_gpu_temperature_celsius{{gpu_id=\"0\",gpu_name=\"H100\"}} 62\n\
         # TYPE all_smi_cpu_utilization gauge\n\
         all_smi_cpu_utilization 12.5\n\
         # TYPE vllm:num_requests_running gauge\n\
         vllm:num_requests_running {running}\n\
         # TYPE vllm:num_requests_waiting gauge\n\
         vllm:num_requests_waiting 0\n\
         # TYPE vllm:gpu_cache_usage_perc gauge\n\
         vllm:gpu_cache_usage_perc 0.25\n\
         # TYPE vllm:num_preemptions_total counter\n\
         vllm:num_preemptions_total {}\n\
         # EOF\n",
        state.preemptions.load(Ordering::Relaxed)
    )
}

async fn models() -> Json<Value> {
    Json(json!({
        "object":"list",
        "data":[{"id":"metrum-ai-bench-cli-mock","object":"model","owned_by":"metrum-ai"}]
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
        telemetry_fixture: args.telemetry_fixture,
        scrapes: AtomicU64::new(0),
    });
    let app = Router::new()
        .route("/health", get(|| async { StatusCode::OK }))
        .route("/healthz", get(|| async { StatusCode::OK }))
        .route("/ready", get(|| async { StatusCode::OK }))
        .route("/metrics", get(metrics))
        // Metrum all-smi fork documents /metric; serve the same body for CI.
        .route("/metric", get(metrics))
        .route("/v1/models", get(models))
        .route("/v1/chat/completions", post(infer))
        .route("/v1/completions", post(infer))
        .route("/v1/embeddings", post(infer))
        .route("/v1/rerank", post(infer))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(&args.listen).await?;
    println!(
        "metrum-ai-bench-cli-mock-server listening on {}",
        args.listen
    );
    axum::serve(listener, app).await?;
    Ok(())
}
