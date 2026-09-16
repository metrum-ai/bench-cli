// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Unified strategic benchmark runner for chat, embeddings and reranking.

use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use metrum_ai_bench::strategic::{
    controlled_messages, detect_knee, export_csv, export_html, export_mlperf, load_sessions,
    now_unix_ns, scrape_metrics, summarize_stage, BenchRecord, MlperfScenario, PrefixControl,
    ServerMetrics, Validity,
};
use serde_json::{json, Value};
#[cfg(feature = "otlp")]
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum EndpointKind {
    Chat,
    Embeddings,
    Rerank,
}

#[derive(Parser, Debug)]
#[command(about = "Sweep and benchmark OpenAI-compatible inference endpoints")]
struct Args {
    #[arg(long)]
    url: String,
    #[arg(long, env = "OPENAI_API_KEY", default_value = "")]
    api_key: String,
    #[arg(long)]
    model: String,
    #[arg(long, value_enum, default_value = "chat")]
    kind: EndpointKind,
    #[arg(long, default_value_t = 100)]
    requests_per_stage: u64,
    #[arg(long, default_value = "1,2,4,8")]
    sweep: String,
    #[arg(long, value_enum, default_value = "concurrency")]
    sweep_by: SweepBy,
    #[arg(
        long,
        default_value_t = 256,
        value_parser = clap::value_parser!(u32).range(1..),
        help = "Maximum outstanding requests during a rate sweep"
    )]
    max_in_flight: u32,
    #[arg(long, default_value = "Hello")]
    prompt: String,
    #[arg(long)]
    sessions: Option<PathBuf>,
    #[arg(long, value_enum, default_value = "shared")]
    prefix_control: PrefixControl,
    #[arg(long)]
    shared_prefix: Option<String>,
    #[arg(long)]
    json_schema: Option<PathBuf>,
    #[arg(long)]
    tools: Option<PathBuf>,
    #[arg(long)]
    metrics_url: Option<String>,
    #[arg(long, default_value_t = 250)]
    metrics_interval_ms: u64,
    #[arg(long, default_value = "metrum-ai-bench-report.html")]
    html: PathBuf,
    #[arg(long, default_value = "metrum-ai-bench-requests.csv")]
    csv: PathBuf,
    #[arg(long)]
    mlperf_dir: Option<PathBuf>,
    #[arg(long, value_enum, default_value = "server")]
    mlperf_scenario: MlperfScenario,
    #[arg(long)]
    otlp_endpoint: Option<String>,
    #[arg(long, default_value = "metrum-ai-bench")]
    otlp_service_name: String,
    #[arg(long, default_value_t = 300)]
    timeout_seconds: u64,
    #[arg(
        long = "slo",
        value_name = "METRIC=SECONDS",
        help = "Repeatable goodput threshold: e2e= (ttft=/tpot= accepted but ignored; strategic records lack those timings)"
    )]
    slos: Vec<String>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum SweepBy {
    Concurrency,
    Rate,
}

#[derive(Clone)]
struct Input {
    body: Value,
    session_id: Option<String>,
    turn: Option<usize>,
}

fn parse_sweep(value: &str) -> Result<Vec<f64>> {
    let stages: Vec<f64> = value
        .split(',')
        .map(|part| {
            let number: f64 = part
                .trim()
                .parse()
                .with_context(|| format!("invalid sweep value {part:?}"))?;
            if !number.is_finite() || number <= 0.0 {
                bail!("sweep values must be finite and greater than zero");
            }
            Ok(number)
        })
        .collect::<Result<_>>()?;
    if stages.is_empty() {
        bail!("sweep must contain at least one value");
    }
    if stages.windows(2).any(|pair| pair[0] >= pair[1]) {
        bail!("sweep values must be strictly increasing");
    }
    Ok(stages)
}

fn read_json(path: &Path) -> Result<Value> {
    serde_json::from_reader(
        std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?,
    )
    .with_context(|| format!("parse {}", path.display()))
}

fn make_inputs(args: &Args, schema: Option<&Value>, tools: Option<&Value>) -> Result<Vec<Input>> {
    if let Some(path) = &args.sessions {
        if !matches!(args.kind, EndpointKind::Chat) {
            bail!("--sessions is only valid for chat endpoints");
        }
        let mut inputs = Vec::new();
        for session in load_sessions(path)? {
            for turn in 1..=session.messages.len() {
                let messages = controlled_messages(
                    &session.messages[..turn],
                    args.shared_prefix.as_deref(),
                    args.prefix_control,
                    &session.session_id,
                );
                let mut body = json!({"model":args.model,"messages":messages});
                add_structured(&mut body, schema, tools);
                inputs.push(Input {
                    body,
                    session_id: Some(session.session_id.clone()),
                    turn: Some(turn),
                });
            }
        }
        return Ok(inputs);
    }
    let mut body = match args.kind {
        EndpointKind::Chat => {
            let messages = controlled_messages(
                &[json!({"role":"user","content":args.prompt})],
                args.shared_prefix.as_deref(),
                args.prefix_control,
                "default",
            );
            json!({"model":args.model,"messages":messages})
        }
        EndpointKind::Embeddings => json!({"model":args.model,"input":args.prompt}),
        EndpointKind::Rerank => {
            let documents: Vec<_> = args.prompt.split('|').map(str::trim).collect();
            json!({"model":args.model,"query":documents.first().copied().unwrap_or(""),"documents":documents.iter().skip(1).collect::<Vec<_>>()})
        }
    };
    add_structured(&mut body, schema, tools);
    Ok(vec![Input {
        body,
        session_id: None,
        turn: None,
    }])
}

fn add_structured(body: &mut Value, schema: Option<&Value>, tools: Option<&Value>) {
    if let Some(schema) = schema {
        body["response_format"] = json!({
            "type":"json_schema",
            "json_schema":{"name":"benchmark_output","strict":true,"schema":schema}
        });
    }
    if let Some(tools) = tools {
        body["tools"] = tools.clone();
        body["tool_choice"] = json!("required");
    }
}

fn response_tokens(kind: EndpointKind, response: &Value) -> (u64, u64) {
    match kind {
        EndpointKind::Chat => (
            response
                .pointer("/usage/prompt_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            response
                .pointer("/usage/completion_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        ),
        EndpointKind::Embeddings => (
            response
                .pointer("/usage/prompt_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            0,
        ),
        EndpointKind::Rerank => (
            response
                .pointer("/usage/total_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            0,
        ),
    }
}

async fn run_stage(
    args: &Args,
    stage: f64,
    inputs: &[Input],
    validator: Option<&Validity>,
    seq: Arc<AtomicU64>,
    client: &reqwest::Client,
) -> Result<(Vec<BenchRecord>, f64)> {
    let concurrency = match args.sweep_by {
        SweepBy::Concurrency => stage.ceil() as usize,
        SweepBy::Rate => args.max_in_flight as usize,
    }
    .max(1);
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let kind = args.kind;
    let start = Instant::now();
    let stage_unix_ns = now_unix_ns();
    let mut handles = Vec::with_capacity(args.requests_per_stage as usize);
    for index in 0..args.requests_per_stage {
        let scheduled_offset = matches!(args.sweep_by, SweepBy::Rate)
            .then(|| Duration::from_secs_f64(index as f64 / stage));
        if let Some(offset) = scheduled_offset {
            tokio::time::sleep(offset.saturating_sub(start.elapsed())).await;
        }
        let permit = semaphore.clone().acquire_owned().await?;
        let sent = Instant::now();
        let sent_unix_ns = now_unix_ns();
        let (scheduled, scheduled_unix_ns) = scheduled_offset
            .map_or((sent, sent_unix_ns), |offset| {
                (start + offset, stage_unix_ns + offset.as_nanos())
            });
        let client = client.clone();
        let input = inputs[index as usize % inputs.len()].clone();
        let url = args.url.clone();
        let api_key = args.api_key.clone();
        let validator = validator.cloned();
        let sequence = seq.fetch_add(1, Ordering::Relaxed);
        handles.push(tokio::spawn(async move {
            let result = client
                .post(&url)
                .bearer_auth(api_key)
                .json(&input.body)
                .send()
                .await;
            let (success, valid, input_tokens, output_tokens, error, completed) = match result {
                Ok(response) => match response.error_for_status() {
                    Ok(response) => match response.json::<Value>().await {
                        Ok(value) => {
                            let completed = Instant::now();
                            let (input_tokens, output_tokens) = response_tokens(kind, &value);
                            (
                                true,
                                validator.as_ref().map(|check| check.validate(&value)),
                                input_tokens,
                                output_tokens,
                                None,
                                completed,
                            )
                        }
                        Err(error) => (false, None, 0, 0, Some(error.to_string()), Instant::now()),
                    },
                    Err(error) => (false, None, 0, 0, Some(error.to_string()), Instant::now()),
                },
                Err(error) => (false, None, 0, 0, Some(error.to_string()), Instant::now()),
            };
            drop(permit);
            BenchRecord {
                seq: sequence,
                stage,
                endpoint: url,
                scheduled_unix_ns,
                sent_unix_ns,
                latency_s: completed.saturating_duration_since(scheduled).as_secs_f64(),
                queue_delay_s: sent.saturating_duration_since(scheduled).as_secs_f64(),
                service_latency_s: completed.saturating_duration_since(sent).as_secs_f64(),
                success,
                valid,
                input_tokens,
                output_tokens,
                session_id: input.session_id,
                turn: input.turn,
                error,
            }
        }));
    }
    let mut records = Vec::with_capacity(handles.len());
    for handle in handles {
        records.push(handle.await.context("request task failed")?);
    }
    Ok((records, start.elapsed().as_secs_f64()))
}

fn aggregate_server(samples: &[ServerMetrics]) -> ServerMetrics {
    let maximum = |select: fn(&ServerMetrics) -> Option<f64>| {
        samples.iter().filter_map(select).max_by(f64::total_cmp)
    };
    ServerMetrics {
        kv_cache_usage: maximum(|sample| sample.kv_cache_usage),
        preemptions: samples
            .last()
            .and_then(|sample| sample.preemptions)
            .zip(samples.first().and_then(|sample| sample.preemptions))
            .map(|(last, first)| (last - first).max(0.0)),
        requests_running: maximum(|sample| sample.requests_running),
        requests_waiting: maximum(|sample| sample.requests_waiting),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let stages = parse_sweep(&args.sweep)?;
    let schema = args.json_schema.as_deref().map(read_json).transpose()?;
    let tools = args.tools.as_deref().map(read_json).transpose()?;
    if schema.is_some() && tools.is_some() {
        bail!("--json-schema and --tools are mutually exclusive");
    }
    let validator = match (&schema, &tools) {
        (Some(schema), None) => Some(Validity::json_schema(schema)?),
        (None, Some(tools)) => Some(Validity::tool_names(tools)?),
        _ => None,
    };
    let inputs = make_inputs(&args, schema.as_ref(), tools.as_ref())?;
    let stop_scraper = Arc::new(AtomicBool::new(false));
    let server_samples = Arc::new(Mutex::new(Vec::new()));
    let scraper = if let Some(url) = args.metrics_url.clone() {
        let stop = stop_scraper.clone();
        let samples = server_samples.clone();
        let interval = Duration::from_millis(args.metrics_interval_ms.max(50));
        Some(tokio::spawn(async move {
            let client = reqwest::Client::new();
            while !stop.load(Ordering::Relaxed) {
                if let Ok(sample) = scrape_metrics(&client, &url).await {
                    samples.lock().await.push(sample);
                }
                tokio::time::sleep(interval).await;
            }
        }))
    } else {
        None
    };
    let sequence = Arc::new(AtomicU64::new(0));
    let slos = metrum_ai_bench::summary::SloConfig::parse(&args.slos)?;
    let redacted_config = json!({
        "url": args.url,
        "model": args.model,
        "kind": format!("{:?}", args.kind).to_ascii_lowercase(),
        "sweep_by": format!("{:?}", args.sweep_by).to_ascii_lowercase(),
        "requests_per_stage": args.requests_per_stage,
        "max_in_flight": args.max_in_flight,
        "timeout_seconds": args.timeout_seconds,
        "slos": args.slos,
        "prefix_control": format!("{:?}", args.prefix_control).to_ascii_lowercase(),
        "json_schema": args.json_schema.is_some(),
        "tools": args.tools.is_some(),
        // Secrets intentionally omitted (api_key never stamped).
    });
    // Warm connection pool across stages (single shared client).
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(args.timeout_seconds))
        .pool_max_idle_per_host(args.max_in_flight as usize)
        .build()?;
    let started = Instant::now();
    let mut all_records = Vec::new();
    let mut points = Vec::new();
    for stage in stages {
        let (records, seconds) = run_stage(
            &args,
            stage,
            &inputs,
            validator.as_ref(),
            sequence.clone(),
            &client,
        )
        .await?;
        points.push(summarize_stage(
            stage,
            &records,
            seconds,
            &slos,
            Some(redacted_config.clone()),
        ));
        all_records.extend(records);
    }
    stop_scraper.store(true, Ordering::Relaxed);
    if let Some(scraper) = scraper {
        scraper.await?;
    }
    let duration_s = started.elapsed().as_secs_f64();
    let server = aggregate_server(&server_samples.lock().await);
    let knee = detect_knee(&points);
    export_csv(&args.csv, &all_records)?;
    export_html(
        &args.html,
        "Metrum AI Bench strategic sweep",
        &points,
        knee,
        &server,
    )?;
    if let Some(directory) = &args.mlperf_dir {
        export_mlperf(directory, args.mlperf_scenario, &all_records, duration_s)?;
    }
    #[cfg(feature = "otlp")]
    if let Some(endpoint) = &args.otlp_endpoint {
        let headers: BTreeMap<String, String> = std::env::var("OTEL_EXPORTER_OTLP_HEADERS")
            .ok()
            .map(|value| {
                value
                    .split(',')
                    .filter_map(|pair| pair.split_once('='))
                    .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
                    .collect()
            })
            .unwrap_or_default();
        metrum_ai_bench::strategic::export_otlp(
            &reqwest::Client::new(),
            endpoint,
            &args.otlp_service_name,
            &all_records,
            &headers,
        )
        .await?;
    }
    #[cfg(not(feature = "otlp"))]
    if args.otlp_endpoint.is_some() {
        bail!("OTLP export requires a build with --features otlp");
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "points": points,
            "knee": knee.map(|index| &points[index]),
            "server_metrics": server,
            "records_csv": args.csv,
            "html_report": args.html,
        }))?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweep_parser_rejects_invalid_values() {
        assert_eq!(parse_sweep("1,2.5,4").unwrap(), vec![1.0, 2.5, 4.0]);
        assert!(parse_sweep("1,0").is_err());
        assert!(parse_sweep("2,1").is_err());
        assert!(parse_sweep("x").is_err());
    }
}
