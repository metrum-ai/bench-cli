// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Unified strategic benchmark runner for chat, embeddings and reranking.

use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use metrum_ai_bench::strategic::{
    controlled_messages, detect_knee, export_csv, export_html, export_mlperf, load_sessions,
    now_unix_ns, scrape_metrics, summarize_stage_with_options, BenchRecord, MlperfScenario,
    PrefixControl, ServerMetrics, Validity,
};
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;
use serde_json::{json, Value};
#[cfg(feature = "otlp")]
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy, Debug, ValueEnum)]
enum EndpointKind {
    Chat,
    Embeddings,
    Rerank,
}

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Sweep and benchmark OpenAI-compatible inference endpoints"
)]
struct Args {
    #[arg(long, help = "Print version information and exit")]
    version_only: bool,
    #[arg(long, required_unless_present = "version_only")]
    url: Option<String>,
    #[arg(long, env = "OPENAI_API_KEY", default_value = "")]
    api_key: String,
    #[arg(long, required_unless_present = "version_only")]
    model: Option<String>,
    #[arg(long, value_enum, default_value = "chat")]
    kind: EndpointKind,
    #[arg(
        long,
        conflicts_with = "tools",
        help = "Stream chat responses to measure TTFT; embeddings and rerank remain JSON"
    )]
    streaming: bool,
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
    #[arg(
        long,
        default_value = "Hello",
        help = "Single prompt string (ignored when --prompts or --sessions is set)"
    )]
    prompt: String,
    #[arg(
        long,
        conflicts_with = "sessions",
        help = "JSONL prompt file or http(s) URL (objects with \"prompt\"); cycles across requests"
    )]
    prompts: Option<String>,
    #[arg(
        long,
        help = "Max completion tokens for chat bodies; required when --prompts is set, recommended for all chat sweeps"
    )]
    max_tokens: Option<u32>,
    #[arg(
        long,
        default_value_t = false,
        help = "Send ignore_eos=true in chat request bodies (engine extension; for fixed-length throughput studies)"
    )]
    ignore_eos: bool,
    #[arg(
        long,
        value_name = "N",
        help = "Send min_tokens=N in chat request bodies (engine extension; must be <= --max-tokens)"
    )]
    min_tokens: Option<u32>,
    #[arg(
        long,
        value_name = "JSON",
        help = "Merge extra JSON object fields into chat request bodies"
    )]
    extra_body_json: Option<String>,
    #[arg(
        long,
        default_value_t = 0,
        help = "Per-stage warmup requests excluded from measured aggregates (cold-start control)"
    )]
    warmup_requests: u64,
    #[arg(
        long,
        default_value_t = 0,
        help = "RNG seed used when --shuffle-prompts is set"
    )]
    seed: u64,
    #[arg(long, help = "Shuffle --prompts with --seed before cycling")]
    shuffle_prompts: bool,
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
    #[arg(
        long,
        value_name = "PATH",
        help = "Tagged NDJSON run log (run/stage/request/telemetry/summary rows)"
    )]
    ndjson: Option<PathBuf>,
    #[arg(long, default_value = "metrum-ai-bench-cli-report.html")]
    html: PathBuf,
    #[arg(long, default_value = "metrum-ai-bench-cli-requests.csv")]
    csv: PathBuf,
    #[arg(long)]
    mlperf_dir: Option<PathBuf>,
    #[arg(long, value_enum, default_value = "server")]
    mlperf_scenario: MlperfScenario,
    #[arg(long)]
    otlp_endpoint: Option<String>,
    #[arg(long, default_value = "metrum-ai-bench-cli")]
    otlp_service_name: String,
    #[arg(long, default_value_t = 300)]
    timeout_seconds: u64,
    #[arg(
        long = "slo",
        value_name = "METRIC=VALUE",
        help = "Repeatable goodput threshold: e2e=, ttft=, tpot= (streaming, seconds); user_tps= (tok/s per in-flight user)"
    )]
    slos: Vec<String>,
    #[arg(
        long,
        value_name = "USD_PER_HOUR",
        help = "Declared platform cost ($/hour); overrides sut.cost.price_per_hour for stage cost_per_million_output_tokens"
    )]
    price_per_hour: Option<f64>,
    #[arg(
        long,
        value_name = "TOKENS",
        help = "Expected input tokens for runtime ISL validation (overrides mix-report)"
    )]
    isl_target: Option<f64>,
    #[arg(
        long,
        value_name = "TOKENS",
        help = "Expected output tokens for runtime OSL validation (overrides mix-report)"
    )]
    osl_target: Option<f64>,
    #[arg(
        long,
        default_value_t = 0.0,
        help = "Allowed absolute deviation from --isl-target (tokens)"
    )]
    isl_tolerance: f64,
    #[arg(
        long,
        default_value_t = 0.0,
        help = "Allowed absolute deviation from --osl-target (tokens)"
    )]
    osl_tolerance: f64,
    #[arg(
        long,
        value_name = "PATH",
        help = "Prompt-library mix report JSON; fills ISL/OSL targets when CLI targets are unset"
    )]
    prompt_mix_report: Option<PathBuf>,
    #[arg(
        long,
        default_value_t = false,
        help = "Exit non-zero when measured OSL mismatches exceed --osl-tolerance"
    )]
    fail_on_osl_mismatch: bool,
    #[arg(
        long,
        value_name = "PATH",
        help = "Operator-declared SUT block (JSON/YAML) embedded in sweep summary and HTML"
    )]
    sut: Option<PathBuf>,
    #[arg(
        long,
        default_value_t = false,
        env = "METRUM_AI_BENCH_REQUIRE_SUT",
        help = "Refuse to run without a complete --sut block (gpu.model, gpu.count, driver_version, runtime.name/version/config, host_os); implies --redact-hostname"
    )]
    require_sut: bool,
    #[arg(
        long,
        default_value_t = false,
        env = "METRUM_AI_BENCH_REDACT_HOSTNAME",
        help = "Reserved for parity with modality binaries (strategic stamps SUT only)"
    )]
    redact_hostname: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum SweepBy {
    Concurrency,
    Rate,
}

#[derive(Clone, Debug)]
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

struct ChatBodyOpts<'a> {
    model: &'a str,
    prompt: &'a str,
    streaming: bool,
    max_tokens: Option<u32>,
    ignore_eos: bool,
    min_tokens: Option<u32>,
    extra_body: Option<&'a Value>,
    shared_prefix: Option<&'a str>,
    prefix_control: PrefixControl,
    session_key: &'a str,
    schema: Option<&'a Value>,
    tools: Option<&'a Value>,
}

fn apply_chat_controls(
    body: &mut Value,
    max_tokens: Option<u32>,
    ignore_eos: bool,
    min_tokens: Option<u32>,
    extra_body: Option<&Value>,
) -> Result<()> {
    if let Some(max_tokens) = max_tokens {
        body["max_tokens"] = json!(max_tokens);
    }
    if ignore_eos {
        body["ignore_eos"] = json!(true);
    }
    if let Some(min_tokens) = min_tokens {
        body["min_tokens"] = json!(min_tokens);
    }
    if let Some(extra) = extra_body {
        let Some(dst) = body.as_object_mut() else {
            bail!("chat body must be a JSON object");
        };
        let Some(src) = extra.as_object() else {
            bail!("--extra-body-json must be a JSON object");
        };
        for (key, value) in src {
            dst.insert(key.clone(), value.clone());
        }
    }
    Ok(())
}

fn chat_body(opts: ChatBodyOpts<'_>) -> Result<Value> {
    let messages = controlled_messages(
        &[json!({"role":"user","content":opts.prompt})],
        opts.shared_prefix,
        opts.prefix_control,
        opts.session_key,
    );
    let mut body = json!({"model":opts.model,"messages":messages,"stream":opts.streaming});
    if opts.streaming {
        body["stream_options"] = json!({"include_usage": true});
    }
    apply_chat_controls(
        &mut body,
        opts.max_tokens,
        opts.ignore_eos,
        opts.min_tokens,
        opts.extra_body,
    )?;
    add_structured(&mut body, opts.schema, opts.tools);
    Ok(body)
}

fn parse_extra_body(raw: Option<&str>) -> Result<Option<Value>> {
    match raw {
        None => Ok(None),
        Some(text) => {
            let value: Value =
                serde_json::from_str(text).context("--extra-body-json must be valid JSON")?;
            if !value.is_object() {
                bail!("--extra-body-json must be a JSON object");
            }
            Ok(Some(value))
        }
    }
}

fn validate_chat_controls(args: &Args) -> Result<()> {
    if (args.ignore_eos || args.min_tokens.is_some() || args.extra_body_json.is_some())
        && !matches!(args.kind, EndpointKind::Chat)
    {
        bail!("--ignore-eos, --min-tokens, and --extra-body-json are only valid for --kind chat");
    }
    if let (Some(min_tokens), Some(max_tokens)) = (args.min_tokens, args.max_tokens) {
        if min_tokens > max_tokens {
            bail!("--min-tokens ({min_tokens}) must be <= --max-tokens ({max_tokens})");
        }
    }
    if args.min_tokens.is_some() && args.max_tokens.is_none() {
        bail!("--min-tokens requires --max-tokens");
    }
    Ok(())
}

fn make_inputs(
    args: &Args,
    model: &str,
    schema: Option<&Value>,
    tools: Option<&Value>,
) -> Result<Vec<Input>> {
    validate_chat_controls(args)?;
    let extra_body = parse_extra_body(args.extra_body_json.as_deref())?;
    if args.prompts.is_some() && args.max_tokens.is_none() {
        bail!("--max-tokens is required when --prompts is set (bounds OSL for comparable sweeps)");
    }
    if matches!(args.kind, EndpointKind::Chat)
        && args.max_tokens.is_none()
        && args.prompts.is_none()
        && args.sessions.is_none()
    {
        eprintln!(
            "warning: chat sweep without --max-tokens; output length is uncontrolled and tok/s is not comparable across configs"
        );
    }
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
                let mut body = json!({"model":model,"messages":messages,"stream":args.streaming});
                if args.streaming {
                    body["stream_options"] = json!({"include_usage": true});
                }
                apply_chat_controls(
                    &mut body,
                    args.max_tokens,
                    args.ignore_eos,
                    args.min_tokens,
                    extra_body.as_ref(),
                )?;
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
    if let Some(prompts_path) = &args.prompts {
        if !matches!(args.kind, EndpointKind::Chat) {
            bail!("--prompts is only valid for chat endpoints");
        }
        let mut prompts =
            metrum_ai_bench::prompt_inputs::load_metrum_ai_bench_llm_prompts(prompts_path)
                .map_err(|err| anyhow::anyhow!("{err}"))?;
        if args.shuffle_prompts {
            let mut rng = StdRng::seed_from_u64(args.seed);
            prompts.shuffle(&mut rng);
        }
        let mut inputs = Vec::with_capacity(prompts.len());
        for (index, prompt) in prompts.iter().enumerate() {
            let session_key = format!("prompt-{index}");
            inputs.push(Input {
                body: chat_body(ChatBodyOpts {
                    model,
                    prompt,
                    streaming: args.streaming,
                    max_tokens: args.max_tokens,
                    ignore_eos: args.ignore_eos,
                    min_tokens: args.min_tokens,
                    extra_body: extra_body.as_ref(),
                    shared_prefix: args.shared_prefix.as_deref(),
                    prefix_control: args.prefix_control,
                    session_key: &session_key,
                    schema,
                    tools,
                })?,
                session_id: None,
                turn: None,
            });
        }
        return Ok(inputs);
    }
    let body = match args.kind {
        EndpointKind::Chat => chat_body(ChatBodyOpts {
            model,
            prompt: &args.prompt,
            streaming: args.streaming,
            max_tokens: args.max_tokens,
            ignore_eos: args.ignore_eos,
            min_tokens: args.min_tokens,
            extra_body: extra_body.as_ref(),
            shared_prefix: args.shared_prefix.as_deref(),
            prefix_control: args.prefix_control,
            session_key: "default",
            schema,
            tools,
        })?,
        EndpointKind::Embeddings => json!({"model":model,"input":args.prompt}),
        EndpointKind::Rerank => {
            let documents: Vec<_> = args.prompt.split('|').map(str::trim).collect();
            json!({"model":model,"query":documents.first().copied().unwrap_or(""),"documents":documents.iter().skip(1).collect::<Vec<_>>()})
        }
    };
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

#[allow(clippy::too_many_arguments)]
fn spawn_one_request(
    args: &Args,
    url: &str,
    stage: f64,
    input: Input,
    validator: Option<&Validity>,
    seq: Arc<AtomicU64>,
    client: &reqwest::Client,
    semaphore: Arc<Semaphore>,
    inflight_tracker: Arc<metrum_ai_bench::concurrency::InFlightTracker>,
    schedule_epoch: Instant,
    schedule_epoch_unix_ns: u128,
    run_epoch: Arc<metrum_ai_bench::telemetry::RunEpoch>,
    run_id: Arc<String>,
    ndjson: Option<metrum_ai_bench::telemetry::NdjsonWriter>,
    scheduled_offset: Option<Duration>,
    warmup: bool,
) -> tokio::task::JoinHandle<BenchRecord> {
    let client = client.clone();
    let url = url.to_string();
    let api_key = args.api_key.clone();
    let validator = validator.cloned();
    let kind = args.kind;
    let streaming = args.streaming && matches!(kind, EndpointKind::Chat);
    let sequence = seq.fetch_add(1, Ordering::Relaxed);
    tokio::spawn(async move {
        let permit = metrum_ai_bench::concurrency::acquire_with_engagement(
            Arc::clone(&semaphore),
            &inflight_tracker,
        )
        .await
        .expect("semaphore closed");
        let inflight_guard = inflight_tracker.guard();
        let in_flight_at_send = inflight_guard.in_flight;
        let sent = Instant::now();
        let t_sent_ns = run_epoch.elapsed_ns();
        let sent_unix_ns = now_unix_ns();
        let (scheduled, scheduled_unix_ns) =
            scheduled_offset.map_or((sent, sent_unix_ns), |offset| {
                (
                    schedule_epoch + offset,
                    schedule_epoch_unix_ns + offset.as_nanos(),
                )
            });
        let t_sched_ns = scheduled
            .checked_duration_since(run_epoch.mono())
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(t_sent_ns);
        let connect_slot = metrum_ai_bench::connect_timing::ConnectSlot::new();
        let result = metrum_ai_bench::connect_timing::with_connect_slot(
            Arc::clone(&connect_slot),
            client
                .post(&url)
                .bearer_auth(api_key)
                .json(&input.body)
                .send(),
        )
        .await;
        let connect_s = connect_slot.take();
        let mut first_byte_s = None;
        let mut t_first_ns = None;
        let mut ttft_s = None;
        let mut itl_s = Vec::new();
        let result: Result<Value> = async {
            let response = result?;
            first_byte_s.replace(sent.elapsed().as_secs_f64());
            t_first_ns.replace(run_epoch.elapsed_ns());
            let response = response.error_for_status()?;
            if streaming {
                let stream =
                    metrum_ai_bench::chat_stream::consume(response.bytes_stream(), sent).await?;
                ttft_s = Some(stream.ttft.as_secs_f64());
                itl_s = stream.itl.iter().map(|d| d.as_secs_f64()).collect();
                Ok(json!({
                    "choices": [{"message": {"role": "assistant", "content": stream.completion_text}}],
                    "usage": {"prompt_tokens": stream.prompt_tokens, "completion_tokens": stream.completion_tokens}
                }))
            } else {
                Ok(response.json::<Value>().await?)
            }
        }
        .await;
        let completed = Instant::now();
        let t_done_ns = run_epoch.elapsed_ns();
        let (success, valid, input_tokens, output_tokens, error) = match result {
            Ok(value) => {
                let (input_tokens, output_tokens) = response_tokens(kind, &value);
                (
                    true,
                    validator.as_ref().map(|check| check.validate(&value)),
                    input_tokens,
                    output_tokens,
                    None,
                )
            }
            Err(error) => (false, None, 0, 0, Some(error.to_string())),
        };
        drop(permit);
        drop(inflight_guard);
        let record = BenchRecord {
            seq: sequence,
            stage,
            endpoint: url,
            scheduled_unix_ns,
            sent_unix_ns,
            latency_s: completed.saturating_duration_since(scheduled).as_secs_f64(),
            queue_delay_s: sent.saturating_duration_since(scheduled).as_secs_f64(),
            service_latency_s: completed.saturating_duration_since(sent).as_secs_f64(),
            first_byte_s,
            connect_s: Some(connect_s),
            ttft_s,
            prefill_s: None,
            decode_s: None,
            decode_tok_s: None,
            itl_s,
            in_flight_at_send: Some(in_flight_at_send),
            success,
            valid,
            input_tokens,
            output_tokens,
            session_id: input.session_id,
            turn: input.turn,
            error: error.clone(),
            warmup,
        }
        .with_phase_metrics();
        if let Some(writer) = ndjson {
            let _ = writer
                .send_priority(metrum_ai_bench::telemetry::Row::Request(
                    metrum_ai_bench::telemetry::RequestRow {
                        run_id: (*run_id).clone(),
                        seq: sequence,
                        stage,
                        warmup,
                        t_sched_ns,
                        t_sent_ns,
                        t_first_ns,
                        t_done_ns,
                        success,
                        input_tokens,
                        output_tokens,
                        latency_s: record.latency_s,
                        queue_delay_s: record.queue_delay_s,
                        service_latency_s: record.service_latency_s,
                        ttft_s: record.ttft_s,
                        error,
                        telemetry_at_done: None,
                    },
                ))
                .await;
        }
        record
    })
}

#[allow(clippy::too_many_arguments)]
async fn run_stage(
    args: &Args,
    url: &str,
    stage: f64,
    inputs: &[Input],
    validator: Option<&Validity>,
    seq: Arc<AtomicU64>,
    client: &reqwest::Client,
    run_epoch: Arc<metrum_ai_bench::telemetry::RunEpoch>,
    run_id: Arc<String>,
    ndjson: Option<metrum_ai_bench::telemetry::NdjsonWriter>,
    stop: &metrum_ai_bench::runner::StopFlag,
) -> Result<(
    Vec<BenchRecord>,
    f64,
    metrum_ai_bench::concurrency::ObservedConcurrency,
)> {
    let concurrency = match args.sweep_by {
        SweepBy::Concurrency => stage.ceil() as usize,
        SweepBy::Rate => args.max_in_flight as usize,
    }
    .max(1);
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let inflight_tracker = Arc::new(metrum_ai_bench::concurrency::InFlightTracker::new(
        concurrency as u32,
    ));
    let mut records =
        Vec::with_capacity(args.warmup_requests.saturating_add(args.requests_per_stage) as usize);

    // Phase 1: fully complete warmup before measurement begins.
    if args.warmup_requests > 0 && !stop.is_stopped() {
        let warmup_epoch = Instant::now();
        let warmup_unix_ns = now_unix_ns();
        let t_start_ns = run_epoch.elapsed_ns();
        let mut warmup_handles = Vec::with_capacity(args.warmup_requests as usize);
        for index in 0..args.warmup_requests {
            if stop.is_stopped() {
                break;
            }
            let input = inputs[index as usize % inputs.len()].clone();
            warmup_handles.push(spawn_one_request(
                args,
                url,
                stage,
                input,
                validator,
                Arc::clone(&seq),
                client,
                Arc::clone(&semaphore),
                Arc::clone(&inflight_tracker),
                warmup_epoch,
                warmup_unix_ns,
                Arc::clone(&run_epoch),
                Arc::clone(&run_id),
                ndjson.clone(),
                None,
                true,
            ));
        }
        for handle in warmup_handles {
            records.push(handle.await.context("warmup request task failed")?);
        }
        let t_end_ns = run_epoch.elapsed_ns();
        if let Some(writer) = &ndjson {
            writer
                .send_priority(metrum_ai_bench::telemetry::Row::Stage(
                    metrum_ai_bench::telemetry::StageRow {
                        run_id: (*run_id).clone(),
                        stage,
                        load: stage,
                        phase: metrum_ai_bench::telemetry::PhaseKind::Warmup,
                        t_start_ns,
                        t_end_ns,
                    },
                ))
                .await?;
        }
    }

    // Phase 2: reset measurement epoch; measured prompts restart at index 0.
    let measure_epoch = Instant::now();
    let measure_unix_ns = now_unix_ns();
    let t_start_ns = run_epoch.elapsed_ns();
    let mut measure_handles = Vec::with_capacity(args.requests_per_stage as usize);
    for index in 0..args.requests_per_stage {
        if stop.is_stopped() {
            break;
        }
        let scheduled_offset = matches!(args.sweep_by, SweepBy::Rate)
            .then(|| Duration::from_secs_f64(index as f64 / stage));
        if let Some(offset) = scheduled_offset {
            tokio::time::sleep(offset.saturating_sub(measure_epoch.elapsed())).await;
        }
        let input = inputs[index as usize % inputs.len()].clone();
        measure_handles.push(spawn_one_request(
            args,
            url,
            stage,
            input,
            validator,
            Arc::clone(&seq),
            client,
            Arc::clone(&semaphore),
            Arc::clone(&inflight_tracker),
            measure_epoch,
            measure_unix_ns,
            Arc::clone(&run_epoch),
            Arc::clone(&run_id),
            ndjson.clone(),
            scheduled_offset,
            false,
        ));
    }
    for handle in measure_handles {
        records.push(handle.await.context("request task failed")?);
    }
    let t_end_ns = run_epoch.elapsed_ns();
    if let Some(writer) = &ndjson {
        writer
            .send_priority(metrum_ai_bench::telemetry::Row::Stage(
                metrum_ai_bench::telemetry::StageRow {
                    run_id: (*run_id).clone(),
                    stage,
                    load: stage,
                    phase: metrum_ai_bench::telemetry::PhaseKind::Measure,
                    t_start_ns,
                    t_end_ns,
                },
            ))
            .await?;
    }

    let measured_seconds = {
        let measured: Vec<_> = records.iter().filter(|r| !r.warmup).collect();
        if let (Some(first), Some(last)) = (measured.first(), measured.last()) {
            let start_ns = first.sent_unix_ns;
            let end_ns = last.sent_unix_ns + ((last.service_latency_s * 1e9) as u128);
            ((end_ns.saturating_sub(start_ns)) as f64 / 1e9).max(f64::EPSILON)
        } else {
            measure_epoch.elapsed().as_secs_f64()
        }
    };
    Ok((records, measured_seconds, inflight_tracker.snapshot()))
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
    if args.version_only {
        println!("metrum-ai-bench-cli-strategic version {VERSION}");
        return Ok(());
    }
    let url = args
        .url
        .clone()
        .ok_or_else(|| anyhow::anyhow!("--url is required"))?;
    let model = args
        .model
        .clone()
        .ok_or_else(|| anyhow::anyhow!("--model is required"))?;
    metrum_ai_bench::sut::warn_remote_benchmark_url(&url);
    let (sut_block, _redact_hostname) = metrum_ai_bench::sut::resolve_sut_flags(
        args.sut.as_deref(),
        args.require_sut,
        args.redact_hostname,
    )?;
    let sut_json = sut_block
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .context("serialize sut")?;
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
    let inputs = make_inputs(&args, &model, schema.as_ref(), tools.as_ref())?;
    let run_epoch = Arc::new(metrum_ai_bench::telemetry::RunEpoch::new());
    let run_id = Arc::new(metrum_ai_bench::unique_id::generate_uuid());
    let stop = metrum_ai_bench::runner::StopFlag::new();
    metrum_ai_bench::runner::install_stop_handlers(stop.clone());
    let (ndjson_writer, ndjson_handle) = match &args.ndjson {
        Some(path) => {
            let (writer, handle) = metrum_ai_bench::telemetry::NdjsonWriter::spawn(path.clone())?;
            (Some(writer), Some(handle))
        }
        None => (None, None),
    };
    if let Some(writer) = &ndjson_writer {
        writer
            .send_priority(metrum_ai_bench::telemetry::Row::Run(
                metrum_ai_bench::telemetry::RunRow {
                    run_id: (*run_id).clone(),
                    t0_wall: run_epoch.t0_wall_iso(),
                    tool_version: VERSION.to_string(),
                    schema_version: metrum_ai_bench::telemetry::TELEMETRY_SCHEMA_VERSION
                        .to_string(),
                    sut: sut_json.clone(),
                    config: json!({
                        "url": url,
                        "model": model,
                        "kind": format!("{:?}", args.kind).to_ascii_lowercase(),
                        "sweep": args.sweep,
                        "sweep_by": format!("{:?}", args.sweep_by).to_ascii_lowercase(),
                        "requests_per_stage": args.requests_per_stage,
                        "warmup_requests": args.warmup_requests,
                        "streaming": args.streaming,
                    }),
                    telemetry_sources: vec![],
                },
            ))
            .await?;
    }
    let stop_scraper = Arc::new(AtomicBool::new(false));
    let server_samples = Arc::new(Mutex::new(Vec::new()));
    let scraper = if let Some(metrics_url) = args.metrics_url.clone() {
        let stop_flag = stop_scraper.clone();
        let samples = server_samples.clone();
        let interval = Duration::from_millis(args.metrics_interval_ms.max(50));
        Some(tokio::spawn(async move {
            let client = reqwest::Client::new();
            while !stop_flag.load(Ordering::Relaxed) {
                if let Ok(sample) = scrape_metrics(&client, &metrics_url).await {
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
    let price =
        metrum_ai_bench::summary::resolve_price_per_hour(args.price_per_hour, sut_block.as_ref());
    let price_per_hour = price.map(|(value, _)| value);
    let redacted_config = json!({
        "url": url,
        "model": model,
        "kind": format!("{:?}", args.kind).to_ascii_lowercase(),
        "sweep_by": format!("{:?}", args.sweep_by).to_ascii_lowercase(),
        "requests_per_stage": args.requests_per_stage,
        "warmup_requests": args.warmup_requests,
        "max_in_flight": args.max_in_flight,
        "max_tokens": args.max_tokens,
        "ignore_eos": args.ignore_eos,
        "min_tokens": args.min_tokens,
        "extra_body_json": args.extra_body_json,
        "prompts": args.prompts,
        "prompt_pool_size": inputs.len(),
        "seed": args.seed,
        "shuffle_prompts": args.shuffle_prompts,
        "timeout_seconds": args.timeout_seconds,
        "slos": args.slos,
        "price_per_hour": price_per_hour,
        "price_provenance": price.map(|(_, provenance)| provenance),
        "prefix_control": format!("{:?}", args.prefix_control).to_ascii_lowercase(),
        "json_schema": args.json_schema.is_some(),
        "tools": args.tools.is_some(),
        "streaming": args.streaming,
        "sut": sut_json,
        "require_sut": args.require_sut,
        "ndjson": args.ndjson,
        // Secrets intentionally omitted (api_key never stamped).
    });
    let redact_hostname = args.redact_hostname || args.require_sut;
    let environment =
        metrum_ai_bench::environment::collect(None, Some(model.clone()), redact_hostname);
    // Warm connection pool across stages (single shared client with connect timing).
    let client = metrum_ai_bench::http_client::build_http_client(
        metrum_ai_bench::http_client::HttpClientOptions {
            request_timeout: Some(Duration::from_secs(args.timeout_seconds)),
            connect_timeout: Duration::from_secs(args.timeout_seconds.clamp(1, 30)),
            pool_max_idle_per_host: args.max_in_flight as usize,
            pool_idle_timeout: Duration::from_secs(90),
            tcp_keepalive: Duration::from_secs(60),
            ca_cert: None,
            insecure: false,
        },
    )?;
    let isl_osl_targets = metrum_ai_bench::isl_osl::IslOslTargets::resolve(
        args.isl_target,
        args.osl_target,
        args.isl_tolerance,
        args.osl_tolerance,
        args.prompt_mix_report.as_deref(),
    )?;
    let started = Instant::now();
    let mut all_records = Vec::new();
    let mut points = Vec::new();
    let mut any_osl_validation = None;
    let mut partial = false;
    for stage in stages {
        if stop.is_stopped() {
            partial = true;
            break;
        }
        let (records, seconds, observed) = run_stage(
            &args,
            &url,
            stage,
            &inputs,
            validator.as_ref(),
            sequence.clone(),
            &client,
            Arc::clone(&run_epoch),
            Arc::clone(&run_id),
            ndjson_writer.clone(),
            &stop,
        )
        .await?;
        if stop.is_stopped() {
            partial = true;
        }
        let token_rows: Vec<(u64, u64)> = records
            .iter()
            .filter(|r| !r.warmup && r.success)
            .map(|r| (r.input_tokens, r.output_tokens))
            .collect();
        let isl_osl =
            metrum_ai_bench::isl_osl::validate_token_counts(&token_rows, &isl_osl_targets);
        if let Some(ref v) = isl_osl {
            any_osl_validation = Some(v.clone());
        }
        points.push(summarize_stage_with_options(
            stage,
            &records,
            seconds,
            &slos,
            Some(redacted_config.clone()),
            price_per_hour,
            Some(observed),
            isl_osl,
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
        sut_json.as_ref(),
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
    let mut ndjson_stats = metrum_ai_bench::telemetry::WriterStats::default();
    if let (Some(writer), Some(handle)) = (ndjson_writer, ndjson_handle) {
        // Drain pending rows so snapshot reflects request/stage counts, then
        // write summary before closing the channel.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let snap = writer.stats_snapshot();
        let dropped = writer.dropped_telemetry_rows();
        writer
            .send_priority(metrum_ai_bench::telemetry::Row::Summary(
                metrum_ai_bench::telemetry::SummaryRow {
                    run_id: (*run_id).clone(),
                    partial,
                    dropped_telemetry_rows: dropped,
                    request_rows: snap.request_rows,
                    telemetry_rows: snap.telemetry_rows,
                    scrape_error_rows: snap.scrape_error_rows,
                    stage_rows: snap.stage_rows,
                },
            ))
            .await?;
        drop(writer);
        ndjson_stats = handle.shutdown().await?;
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema_version": "metrum-ai-bench-cli.strategic.v1",
            "tool_version": VERSION,
            "environment": environment,
            "config": redacted_config,
            "points": points,
            "knee": knee.map(|index| &points[index]),
            "server_metrics": server,
            "sut": sut_json,
            "records_csv": args.csv,
            "html_report": args.html,
            "partial": partial,
            "ndjson": args.ndjson,
            "dropped_telemetry_rows": ndjson_stats.dropped_telemetry_rows,
        }))?
    );
    if let Some(ref validation) = any_osl_validation {
        metrum_ai_bench::isl_osl::enforce_osl_gate(validation, args.fail_on_osl_mismatch)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_require_max_tokens() {
        let args = Args::try_parse_from([
            "bench",
            "--url",
            "http://localhost",
            "--model",
            "dummy",
            "--prompts",
            "/tmp/missing.jsonl",
        ])
        .expect("args");
        let err = make_inputs(&args, "dummy", None, None).expect_err("max-tokens required");
        assert!(err.to_string().contains("--max-tokens"));
    }

    #[test]
    fn chat_body_includes_max_tokens() {
        let args = Args::try_parse_from([
            "bench",
            "--url",
            "http://localhost",
            "--model",
            "dummy",
            "--max-tokens",
            "32",
            "--streaming",
        ])
        .expect("args");
        let inputs = make_inputs(&args, "dummy", None, None).expect("inputs");
        assert_eq!(inputs[0].body["max_tokens"], 32);
        assert_eq!(inputs[0].body["stream"], true);
    }

    #[test]
    fn chat_body_includes_ignore_eos_and_min_tokens() {
        let args = Args::try_parse_from([
            "bench",
            "--url",
            "http://localhost",
            "--model",
            "dummy",
            "--max-tokens",
            "64",
            "--ignore-eos",
            "--min-tokens",
            "64",
            "--extra-body-json",
            r#"{"temperature":0.0}"#,
        ])
        .expect("args");
        let inputs = make_inputs(&args, "dummy", None, None).expect("inputs");
        assert_eq!(inputs[0].body["ignore_eos"], true);
        assert_eq!(inputs[0].body["min_tokens"], 64);
        assert_eq!(inputs[0].body["temperature"], 0.0);
    }

    #[test]
    fn min_tokens_must_not_exceed_max_tokens() {
        let args = Args::try_parse_from([
            "bench",
            "--url",
            "http://localhost",
            "--model",
            "dummy",
            "--max-tokens",
            "16",
            "--min-tokens",
            "32",
        ])
        .expect("args");
        let err = make_inputs(&args, "dummy", None, None).expect_err("min>max");
        assert!(err.to_string().contains("--min-tokens"));
    }

    #[test]
    fn ignore_eos_rejected_for_embeddings() {
        let args = Args::try_parse_from([
            "bench",
            "--url",
            "http://localhost",
            "--model",
            "dummy",
            "--kind",
            "embeddings",
            "--ignore-eos",
        ])
        .expect("args");
        let err = make_inputs(&args, "dummy", None, None).expect_err("chat-only");
        assert!(err.to_string().contains("chat"));
    }

    #[test]
    fn streaming_only_changes_chat_inputs() {
        for kind in ["chat", "embeddings", "rerank"] {
            for streaming in [false, true] {
                let mut flags = vec![
                    "bench",
                    "--url",
                    "http://localhost",
                    "--model",
                    "dummy",
                    "--kind",
                    kind,
                ];
                if streaming {
                    flags.push("--streaming");
                }
                let args = Args::try_parse_from(flags).expect("args");
                let inputs = make_inputs(&args, "dummy", None, None).expect("inputs");
                let body = &inputs[0].body;
                if kind == "chat" {
                    assert_eq!(body["stream"], streaming);
                    assert_eq!(
                        body["stream_options"]["include_usage"].as_bool(),
                        streaming.then_some(true)
                    );
                } else {
                    assert!(body.get("stream").is_none());
                    assert!(body.get("stream_options").is_none());
                }
            }
        }
    }

    #[test]
    fn sweep_parser_rejects_invalid_values() {
        assert_eq!(parse_sweep("1,2.5,4").unwrap(), vec![1.0, 2.5, 4.0]);
        assert!(parse_sweep("1,0").is_err());
        assert!(parse_sweep("2,1").is_err());
        assert!(parse_sweep("x").is_err());
    }
}
