// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0
#![allow(clippy::too_many_arguments)]

use base64::Engine;
use chrono::{DateTime, Utc};
use clap::Parser;
use image::GenericImageView;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::error::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum ResponseFormat {
    #[value(name = "b64_json", alias = "b64-json")]
    B64Json,
    #[value(name = "url")]
    Url,
}

impl std::fmt::Display for ResponseFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResponseFormat::B64Json => write!(f, "b64_json"),
            ResponseFormat::Url => write!(f, "url"),
        }
    }
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum SeedMode {
    Fixed,
    Increment,
    Prompt,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum LoadBalancer {
    RoundRobin,
    LeastInflight,
    Random,
    WeightedRoundRobin,
}

#[derive(Parser, Debug, Clone)]
#[command(author, version, about = "OpenAI-compatible image generation benchmark", long_about = None)]
struct Args {
    #[arg(long, help = "Print version information and exit")]
    version_only: bool,

    #[arg(
        long,
        default_value_t = false,
        help = "Suppress ASCII banner art if printed (one-line identity). Also set NO_BANNER=1."
    )]
    quiet: bool,

    #[arg(
        long,
        default_value_t = false,
        help = "Opt-in NTP clock check; records offset when available (does not hard-fail)"
    )]
    ntp_check: bool,

    #[arg(long)]
    scenario: String,

    #[arg(
        long,
        requires = "api_key",
        help = "OpenAI-compatible base URL, usually ending in /v1. Required with --api-key."
    )]
    url: Option<String>,

    #[arg(
        long,
        requires = "url",
        help = "API key sent as a Bearer token. Required with --url. Use any placeholder such as \"dummy\" for servers that do not check it. Use --endpoints-file for multiple endpoints."
    )]
    api_key: Option<String>,

    #[arg(long, help = "Repeatable endpoint URL for multi-endpoint mode")]
    endpoint: Vec<String>,

    #[arg(long, help = "JSON or JSONL endpoint file")]
    endpoints_file: Option<String>,

    #[arg(long, value_enum, default_value_t = LoadBalancer::RoundRobin)]
    load_balancer: LoadBalancer,

    #[arg(long, default_value_t = false)]
    endpoint_health_check: bool,

    #[arg(long, default_value = "/models")]
    health_path: String,

    #[arg(long, default_value_t = 2)]
    max_endpoint_failures: usize,

    #[arg(long, default_value_t = 0)]
    endpoint_retry_attempts: usize,

    #[arg(long, default_value_t = 100)]
    endpoint_retry_backoff_ms: u64,

    #[arg(long)]
    model: String,

    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    num_requests: u32,

    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    concurrency: u32,

    #[arg(long, help = "Open-loop request rate (requests/second)")]
    request_rate: Option<f64>,

    #[arg(long, default_value = "constant", value_parser = ["constant", "poisson"])]
    arrival: String,

    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    max_concurrency: Option<u32>,

    #[arg(long)]
    prompt: Option<String>,

    #[arg(long)]
    prompts: Option<String>,

    #[arg(long, default_value = "prompt")]
    prompt_field: String,

    #[arg(long, default_value = "id")]
    id_field: String,

    #[arg(long, default_value_t = false)]
    shuffle_prompts: bool,

    #[arg(
        long,
        default_value_t = 0,
        help = "Warmup requests excluded from summary stats"
    )]
    warmup_requests: u32,

    #[arg(long)]
    seed: Option<i64>,

    #[arg(long, value_enum, default_value_t = SeedMode::Increment)]
    seed_mode: SeedMode,

    #[arg(long, value_parser = clap::value_parser!(u32).range(1..), default_value_t = 1)]
    n: u32,

    #[arg(long, default_value = "1024x1024")]
    size: String,

    #[arg(long, value_enum, default_value_t = ResponseFormat::B64Json)]
    response_format: ResponseFormat,

    #[arg(long)]
    negative_prompt: Option<String>,

    #[arg(long)]
    num_inference_steps: Option<u32>,

    #[arg(long)]
    guidance_scale: Option<f64>,

    #[arg(long)]
    true_cfg_scale: Option<f64>,

    #[arg(
        long,
        help = "Extra JSON object merged into the request body, e.g. '{\"reasoning_effort\":\"medium\"}'. Recorded in the run manifest. See docs/REASONING_MODELS.md."
    )]
    extra_body_json: Option<String>,

    #[arg(
        long,
        help = "Path to a JSON object file merged into the request body (alternative to --extra-body-json). Recorded via the merged body template."
    )]
    extra_body_file: Option<String>,

    #[arg(long, default_value = "300")]
    request_timeout: u64,

    #[arg(long, default_value = "30")]
    connect_timeout: u64,

    #[arg(long, default_value = "60")]
    pool_idle_timeout: u64,

    #[arg(long, default_value = "60")]
    tcp_keepalive: u64,

    #[arg(
        long,
        value_name = "PATH",
        help = "Additional PEM CA certificate for TLS (private gateways)"
    )]
    ca_cert: Option<String>,

    #[arg(
        long,
        default_value_t = false,
        help = "Disable TLS certificate verification (opt-in; stamped into config)"
    )]
    insecure: bool,

    #[arg(long, default_value = "metrum-ai-bench-cli-imagegen-artifacts")]
    artifact_dir: String,

    #[arg(long)]
    data_log: String,

    #[arg(
        long,
        help = "Optional path to write summary.v3 JSON (same schema as the data-log summary line)"
    )]
    summary_json: Option<String>,

    #[arg(long, default_value = "debug.log")]
    debug_log: String,

    #[arg(long, default_value = "error.log")]
    error_log: String,

    #[arg(long, default_value_t = false)]
    save_response_json: bool,

    #[arg(long, default_value_t = false)]
    no_save_images: bool,

    #[arg(long, default_value_t = false)]
    overwrite_artifacts: bool,

    #[arg(
        long,
        default_value_t = false,
        help = "Exit non-zero if any measured request failed (default: exit 0 after writing results)"
    )]
    fail_on_error: bool,

    #[arg(
        long,
        value_name = "USD_PER_HOUR",
        help = "Declared platform cost ($/hour); overrides sut.cost.price_per_hour for cost_per_million_output_tokens"
    )]
    price_per_hour: Option<f64>,

    #[arg(
        long,
        value_name = "PATH",
        help = "Operator-declared SUT block (JSON/YAML) embedded in summary.v3 as sut"
    )]
    sut: Option<std::path::PathBuf>,

    #[arg(
        long,
        default_value_t = false,
        env = "METRUM_AI_BENCH_REQUIRE_SUT",
        help = "Refuse to run without a valid --sut block; implies --redact-hostname"
    )]
    require_sut: bool,

    #[arg(
        long,
        default_value_t = false,
        env = "METRUM_AI_BENCH_REDACT_HOSTNAME",
        help = "Write environment.hostname as null"
    )]
    redact_hostname: bool,
}

#[derive(Clone, Debug)]
struct Endpoint {
    name: String,
    url: String,
    api_key: String,
    weight: u32,
}

#[derive(Debug, Deserialize)]
struct EndpointFile {
    endpoints: Vec<EndpointRecord>,
}

#[derive(Debug, Deserialize)]
struct EndpointRecord {
    name: Option<String>,
    url: String,
    api_key: Option<String>,
    weight: Option<u32>,
}

#[derive(Clone, Debug)]
struct PromptRow {
    id: String,
    prompt: String,
    seed: Option<i64>,
    size: Option<String>,
    negative_prompt: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct ImageArtifact {
    index: usize,
    path: String,
    sha256: String,
    bytes: usize,
    mime_type: String,
    width: u32,
    height: u32,
}

#[derive(Clone, Debug, Serialize)]
struct AttemptRecord {
    attempt: usize,
    endpoint_name: String,
    http_status: Option<u16>,
    latency_ms: f64,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct RequestOutcome {
    request_id: String,
    prompt_id: String,
    prompt_sha256: String,
    endpoint_name: String,
    endpoint_url: String,
    started_at: DateTime<Utc>,
    completed_at: DateTime<Utc>,
    latency_ms: f64,
    /// Headers-received elapsed from send Instant (seconds), when measured.
    first_byte_s: Option<f64>,
    status: String,
    http_status: Option<u16>,
    n_requested: u32,
    n_returned: u32,
    size: String,
    response_format: String,
    seed: Option<i64>,
    image_artifacts: Vec<ImageArtifact>,
    response_bytes: usize,
    attempts: Vec<AttemptRecord>,
    error_type: Option<String>,
    error_message: Option<String>,
}

#[derive(Default, Clone, Debug)]
struct EndpointRuntime {
    inflight: Arc<AtomicUsize>,
    failures: Arc<AtomicUsize>,
    ejected_until: Arc<StdMutex<Option<Instant>>>,
}

#[derive(Default, Debug)]
struct Metrics {
    outcomes: Vec<RequestOutcome>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let args = Args::parse();
    if args.version_only {
        println!("metrum-ai-bench-cli-imagegen version {}", VERSION);
        return Ok(());
    }
    metrum_ai_bench::banner::print_banner(VERSION, "metrum-ai-bench-cli-imagegen", args.quiet);
    let ntp_offset_ms = if args.ntp_check {
        let offset = metrum_ai_bench::timecheck::check_ntp_offset();
        if let Some(offset_ms) = offset {
            println!("NTP clock offset: {}ms", offset_ms);
        }
        offset
    } else {
        None
    };
    validate_args(&args)?;
    prepare_artifacts(&args)?;

    let (sut_block, redact_hostname) = metrum_ai_bench::sut::resolve_sut_flags(
        args.sut.as_deref(),
        args.require_sut,
        args.redact_hostname,
    )?;

    let endpoints = resolve_endpoints(&args)?;
    metrum_ai_bench::sut::warn_remote_benchmark_urls(endpoints.iter().map(|ep| ep.url.as_str()));
    let runtime = endpoints
        .iter()
        .map(|ep| (ep.name.clone(), EndpointRuntime::default()))
        .collect::<HashMap<_, _>>();
    let runtime = Arc::new(runtime);

    let client = metrum_ai_bench::http_client::build_http_client(
        metrum_ai_bench::http_client::HttpClientOptions {
            request_timeout: None,
            connect_timeout: Duration::from_secs(args.connect_timeout),
            pool_max_idle_per_host: args.concurrency as usize,
            pool_idle_timeout: Duration::from_secs(args.pool_idle_timeout),
            tcp_keepalive: Duration::from_secs(args.tcp_keepalive),
            ca_cert: args.ca_cert.as_deref().map(std::path::Path::new),
            insecure: args.insecure,
        },
    )?;

    if args.endpoint_health_check {
        health_check_endpoints(&client, &endpoints, &args.health_path, args.request_timeout)
            .await?;
    }

    let prompts = Arc::new(load_prompts(&args)?);
    let sink = Arc::new(metrum_ai_bench::jsonl::JsonlSink::create(&args.data_log)?);
    let error_log = Arc::new(Mutex::new(
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&args.error_log)?,
    ));
    let metrics = Arc::new(Mutex::new(Metrics::default()));
    let rr = Arc::new(AtomicUsize::new(0));
    let sem = Arc::new(Semaphore::new(
        args.max_concurrency.unwrap_or(args.concurrency) as usize,
    ));
    let run_start = Instant::now();
    let run_id = metrum_ai_bench::unique_id::generate_uuid();
    let stop = metrum_ai_bench::runner::StopFlag::new();
    metrum_ai_bench::runner::install_stop_handlers(stop.clone());

    let mut handles = Vec::new();
    use rand::SeedableRng;
    let mut arrival_rng =
        rand::rngs::StdRng::seed_from_u64(args.seed.unwrap_or_default() as u64 ^ 0x9e37_79b9);
    let arrival_kind = match (args.request_rate, args.arrival.as_str()) {
        (None, _) => metrum_ai_bench::load::ArrivalKind::ClosedLoop,
        (Some(_), "poisson") => metrum_ai_bench::load::ArrivalKind::Poisson,
        (Some(_), _) => metrum_ai_bench::load::ArrivalKind::Constant,
    };
    let (record_tx, mut record_rx) =
        tokio::sync::mpsc::unbounded_channel::<metrum_ai_bench::record::RequestRecord>();
    let slots = metrum_ai_bench::load::schedule(
        arrival_kind,
        u64::from(args.num_requests),
        args.request_rate.unwrap_or(0.0),
        &mut arrival_rng,
    );
    for slot in slots {
        if stop.is_stopped() {
            break;
        }
        let wait = slot.scheduled_delay.saturating_sub(run_start.elapsed());
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        let request_index = slot.seq as usize;
        let permit = sem.clone().acquire_owned().await?;
        let queue_delay = metrum_ai_bench::runner::queue_delay_for_slot(
            arrival_kind,
            run_start.elapsed(),
            slot.scheduled_delay,
        );
        let record_schedule = metrum_ai_bench::runner::should_record_schedule(arrival_kind);
        let scheduled_delay = slot.scheduled_delay;
        let phase = metrum_ai_bench::record::Phase::for_seq(slot.seq, args.warmup_requests);
        let args = args.clone();
        let endpoints = endpoints.clone();
        let runtime = runtime.clone();
        let client = client.clone();
        let prompts = prompts.clone();
        let sink_task = sink.clone();
        let error_log = error_log.clone();
        let metrics = metrics.clone();
        let rr = rr.clone();
        let record_tx = record_tx.clone();
        let run_id_task = run_id.clone();
        let send_offset = run_start.elapsed();
        handles.push(tokio::spawn(async move {
            let _permit = permit;
            let outcome = run_logical_request(
                request_index,
                &args,
                &client,
                &endpoints,
                &runtime,
                &prompts,
                &rr,
            )
            .await;
            if outcome.status != "success" {
                let _ = write_error(
                    &error_log,
                    &format!(
                        "{} {} {}",
                        outcome.request_id,
                        outcome.error_type.clone().unwrap_or_default(),
                        outcome.error_message.clone().unwrap_or_default()
                    ),
                )
                .await;
            }

            let latency = Duration::from_secs_f64(outcome.latency_ms / 1000.0);
            let mut record = if outcome.status == "success" {
                let mut rec = metrum_ai_bench::record::RequestRecord::success(
                    slot.seq,
                    phase,
                    outcome.endpoint_name.clone(),
                    outcome.started_at,
                    metrum_ai_bench::runner::completed_at_from_start(outcome.started_at, latency),
                    latency,
                    None,
                    None,
                    Vec::new(),
                    0,
                    0,
                    0,
                );
                if let Some(fb) = outcome.first_byte_s {
                    rec = rec.with_first_byte(Duration::from_secs_f64(fb));
                }
                rec
            } else {
                let request_error = match outcome.error_type.as_deref() {
                    Some("timeout") => metrum_ai_bench::error::RequestError::Timeout,
                    Some("connect") | Some("connection_error") => {
                        metrum_ai_bench::error::RequestError::Connect
                    }
                    _ => outcome
                        .http_status
                        .map(metrum_ai_bench::error::RequestError::from_status)
                        .unwrap_or_else(|| metrum_ai_bench::error::RequestError::Other {
                            message: outcome
                                .error_message
                                .clone()
                                .unwrap_or_else(|| outcome.status.clone()),
                        }),
                };
                metrum_ai_bench::record::RequestRecord::failed(
                    slot.seq,
                    phase,
                    outcome.endpoint_name.clone(),
                    outcome.started_at,
                    metrum_ai_bench::runner::completed_at_from_start(outcome.started_at, latency),
                    latency,
                    request_error,
                )
            };
            record = record.with_send_offset(send_offset);
            if record_schedule {
                record = record.with_schedule(scheduled_delay, queue_delay);
            }
            record
                .modality_metrics
                .insert("images_requested".into(), f64::from(outcome.n_requested));
            record
                .modality_metrics
                .insert("images_returned".into(), f64::from(outcome.n_returned));
            record
                .modality_metrics
                .insert("response_bytes".into(), outcome.response_bytes as f64);
            for (idx, artifact) in outcome.image_artifacts.iter().enumerate() {
                record
                    .modality_metrics
                    .insert(format!("artifact_{idx}_bytes"), artifact.bytes as f64);
                record
                    .modality_labels
                    .insert(format!("artifact_{idx}_sha256"), artifact.sha256.clone());
            }
            record = record.with_run_id(run_id_task);
            if let Err(e) = sink_task.write(&record) {
                let _ =
                    write_error(&error_log, &format!("request_record_write_error: {}", e)).await;
            }
            let _ = record_tx.send(record);
            metrics.lock().await.outcomes.push(outcome);
        }));
    }
    drop(record_tx);

    let mut shared_records = Vec::new();
    while let Some(rec) = record_rx.recv().await {
        shared_records.push(rec);
    }
    for h in handles {
        h.await?;
    }

    let metrics = metrics.lock().await;
    let window_seconds = metrum_ai_bench::runner::window_seconds_from_records(&shared_records);
    let window_seconds = if window_seconds > 0.0 {
        window_seconds
    } else {
        run_start.elapsed().as_secs_f64()
    };
    let mut shared_summary = metrum_ai_bench::summary::RunSummary::from_records(
        &shared_records,
        window_seconds,
        stop.is_stopped(),
    )
    .with_config(metrum_ai_bench::summary::EffectiveRunConfig {
        run_id: run_id.clone(),
        effective_max_concurrency: args.max_concurrency.unwrap_or(args.concurrency),
        common: metrum_ai_bench::args_common::EffectiveCommonArgs {
            scenario: Some(args.scenario.clone()),
            seed: args.seed.unwrap_or(0) as u64,
            warmup_requests: args.warmup_requests,
            request_rate: args.request_rate,
            arrival: args.arrival.clone(),
            max_concurrency: args.max_concurrency,
            load_balancer: match args.load_balancer {
                LoadBalancer::LeastInflight => {
                    metrum_ai_bench::args_common::LoadBalancer::LeastInflight
                }
                _ => metrum_ai_bench::args_common::LoadBalancer::RoundRobin,
            },
            ignore_eos: false,
            min_tokens: None,
            extra_body_json: args.extra_body_json.clone(),
            system_prompt: None,
            unique_prompts: false,
            tokenizer: None,
            slos: vec![],
            throughput_bin_seconds: 10.0,
            insecure: args.insecure,
            ca_cert: args.ca_cert.clone(),
            fail_on_error: args.fail_on_error,
            price_per_hour: args.price_per_hour,
            sut: args.sut.as_ref().map(|p| p.display().to_string()),
            require_sut: args.require_sut,
            redact_hostname: args.redact_hostname || args.require_sut,
        },
        effective_system_prompt: None,
        body_template: json!({
            "model": args.model,
            "prompt": "{{prompt}}",
            "n": args.n,
            "size": args.size,
            "response_format": args.response_format.to_string(),
            "seed": args.seed,
            "negative_prompt": args.negative_prompt,
            "num_inference_steps": args.num_inference_steps,
            "guidance_scale": args.guidance_scale,
            "true_cfg_scale": args.true_cfg_scale,
        }),
        unique_prompt_nonce_template: None,
        modality: [
            ("size".into(), json!(args.size.clone())),
            ("n".into(), json!(args.n)),
            (
                "response_format".into(),
                json!(args.response_format.to_string()),
            ),
        ]
        .into_iter()
        .collect(),
    });
    shared_summary.environment = metrum_ai_bench::environment::collect(
        ntp_offset_ms,
        Some(args.model.clone()),
        redact_hostname,
    );
    let price =
        metrum_ai_bench::summary::resolve_price_per_hour(args.price_per_hour, sut_block.as_ref());
    shared_summary = shared_summary.with_sut(sut_block).with_price(price);
    sink.write(&shared_summary)?;
    shared_summary.print_console();
    if let Some(path) = &args.summary_json {
        fs::write(path, serde_json::to_string_pretty(&shared_summary)?)?;
    }
    let failed = metrics
        .outcomes
        .iter()
        .filter(|o| {
            o.request_id
                .parse::<u32>()
                .ok()
                .map(|n| n > args.warmup_requests)
                .unwrap_or(true)
                && o.status != "success"
        })
        .count();
    if args.fail_on_error && failed > 0 {
        return Err(anyhow::anyhow!("Test completed with {failed} errors").into());
    }
    Ok(())
}

fn validate_args(args: &Args) -> Result<(), Box<dyn Error + Send + Sync>> {
    if args.prompt.is_none() && args.prompts.is_none() {
        return Err(anyhow::anyhow!("provide --prompt or --prompts").into());
    }
    if args.url.is_some() && !args.endpoint.is_empty() {
        return Err(anyhow::anyhow!("cannot combine --url and --endpoint").into());
    }
    if args.url.is_some() && args.endpoints_file.is_some() {
        return Err(anyhow::anyhow!("cannot combine --url and --endpoints-file").into());
    }
    if !args.endpoint.is_empty() && args.endpoints_file.is_some() {
        return Err(anyhow::anyhow!("cannot combine --endpoint and --endpoints-file").into());
    }
    if parse_size(&args.size).is_none() {
        return Err(anyhow::anyhow!("--size must be formatted as WxH").into());
    }
    Ok(())
}

fn prepare_artifacts(args: &Args) -> Result<(), Box<dyn Error + Send + Sync>> {
    let artifact_dir = Path::new(&args.artifact_dir);
    if artifact_dir.exists() && args.overwrite_artifacts {
        fs::remove_dir_all(artifact_dir)?;
    }
    fs::create_dir_all(artifact_dir)?;
    if args.save_response_json {
        fs::create_dir_all(artifact_dir.join("responses"))?;
    }
    File::create(&args.debug_log)?;
    Ok(())
}

fn resolve_endpoints(args: &Args) -> Result<Vec<Endpoint>, Box<dyn Error + Send + Sync>> {
    if let Some(url) = &args.url {
        return Ok(vec![Endpoint {
            name: endpoint_name(url, 0),
            url: normalize_base_url(url),
            api_key: args
                .api_key
                .clone()
                .unwrap_or_else(|| "dummy-api-key".to_string()),
            weight: 1,
        }]);
    }
    if !args.endpoint.is_empty() {
        return Ok(args
            .endpoint
            .iter()
            .enumerate()
            .map(|(i, url)| Endpoint {
                name: endpoint_name(url, i),
                url: normalize_base_url(url),
                api_key: args
                    .api_key
                    .clone()
                    .unwrap_or_else(|| "dummy-api-key".to_string()),
                weight: 1,
            })
            .collect());
    }
    if let Some(path) = &args.endpoints_file {
        let raw = fs::read_to_string(path)?;
        let records = if raw.trim_start().starts_with('{') {
            serde_json::from_str::<EndpointFile>(&raw)?.endpoints
        } else if raw.trim_start().starts_with('[') {
            serde_json::from_str::<Vec<EndpointRecord>>(&raw)?
        } else {
            raw.lines()
                .filter(|line| !line.trim().is_empty())
                .map(serde_json::from_str::<EndpointRecord>)
                .collect::<Result<Vec<_>, _>>()?
        };
        if records.is_empty() {
            return Err(anyhow::anyhow!("endpoints file contains no endpoints").into());
        }
        return Ok(records
            .into_iter()
            .enumerate()
            .map(|(i, rec)| Endpoint {
                name: rec.name.unwrap_or_else(|| endpoint_name(&rec.url, i)),
                url: normalize_base_url(&rec.url),
                api_key: rec.api_key.unwrap_or_else(|| {
                    args.api_key
                        .clone()
                        .unwrap_or_else(|| "dummy-api-key".to_string())
                }),
                weight: rec.weight.unwrap_or(1).max(1),
            })
            .collect());
    }
    Err(anyhow::anyhow!("provide --url, --endpoint, or --endpoints-file").into())
}

fn normalize_base_url(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}

/// Accept either an OpenAI base URL (`.../v1`) or a full generations path
/// (`.../v1/images/generations`).
fn generations_url(endpoint_url: &str) -> String {
    let trimmed = endpoint_url.trim_end_matches('/');
    if trimmed.ends_with("/images/generations") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/images/generations")
    }
}

fn endpoint_name(url: &str, idx: usize) -> String {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url)
        .split('/')
        .next()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("endpoint-{}", idx))
}

async fn health_check_endpoints(
    client: &Client,
    endpoints: &[Endpoint],
    health_path: &str,
    request_timeout: u64,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    for ep in endpoints {
        let url = format!("{}{}", ep.url, health_path);
        let resp = client
            .get(&url)
            .bearer_auth(&ep.api_key)
            .timeout(Duration::from_secs(request_timeout))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(anyhow::anyhow!(
                "endpoint {} health check failed with {}",
                ep.name,
                resp.status()
            )
            .into());
        }
    }
    Ok(())
}

fn load_prompts(args: &Args) -> Result<Vec<PromptRow>, Box<dyn Error + Send + Sync>> {
    if let Some(prompt) = &args.prompt {
        return Ok(vec![PromptRow {
            id: "prompt-0".to_string(),
            prompt: prompt.clone(),
            seed: args.seed,
            size: None,
            negative_prompt: args.negative_prompt.clone(),
        }]);
    }
    let path = args.prompts.as_ref().unwrap();
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut rows = Vec::new();
    for (idx, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(&line)?;
        let prompt = v
            .get(&args.prompt_field)
            .and_then(Value::as_str)
            .ok_or_else(|| {
                anyhow::anyhow!("prompt row {} missing field {}", idx, args.prompt_field)
            })?
            .to_string();
        let id = v
            .get(&args.id_field)
            .and_then(Value::as_str)
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("prompt-{}", idx));
        rows.push(PromptRow {
            id,
            prompt,
            seed: v.get("seed").and_then(Value::as_i64),
            size: v.get("size").and_then(Value::as_str).map(|s| s.to_string()),
            negative_prompt: v
                .get("negative_prompt")
                .and_then(Value::as_str)
                .map(|s| s.to_string()),
        });
    }
    if rows.is_empty() {
        return Err(anyhow::anyhow!("no prompts loaded from {}", path).into());
    }
    if args.shuffle_prompts {
        use rand::rngs::StdRng;
        use rand::seq::SliceRandom;
        use rand::SeedableRng;
        let seed = args.seed.unwrap_or(0) as u64;
        rows.shuffle(&mut StdRng::seed_from_u64(seed));
    }
    Ok(rows)
}

async fn run_logical_request(
    request_index: usize,
    args: &Args,
    client: &Client,
    endpoints: &[Endpoint],
    runtime: &HashMap<String, EndpointRuntime>,
    prompts: &[PromptRow],
    rr: &AtomicUsize,
) -> RequestOutcome {
    let prompt = prompts[request_index % prompts.len()].clone();
    let request_id = format!("{:06}", request_index + 1);
    let seed = resolve_seed(args, &prompt, request_index);
    let size = prompt.size.clone().unwrap_or_else(|| args.size.clone());
    let prompt_sha256 = hex_sha256(prompt.prompt.as_bytes());
    let started_at = Utc::now();
    let mut attempts = Vec::new();
    let mut last_error_type = None;
    let mut last_error_message = None;

    for attempt_index in 0..=args.endpoint_retry_attempts {
        let random_index = request_index
            .wrapping_mul(0x9e37_79b1)
            .wrapping_add(attempt_index)
            .wrapping_add(args.seed.unwrap_or_default() as usize);
        let ep = choose_endpoint(args.load_balancer, endpoints, runtime, rr, random_index);
        let ep_rt = runtime.get(&ep.name).expect("endpoint runtime exists");
        ep_rt.inflight.fetch_add(1, Ordering::SeqCst);
        let attempt_start = Instant::now();
        let result = make_image_request(
            request_index,
            attempt_index,
            args,
            client,
            &ep,
            &prompt,
            seed,
            &size,
        )
        .await;
        ep_rt.inflight.fetch_sub(1, Ordering::SeqCst);
        match result {
            Ok((status, artifacts, response_bytes, n_returned, first_byte, service_latency)) => {
                let latency_ms = service_latency.as_secs_f64() * 1000.0;
                attempts.push(AttemptRecord {
                    attempt: attempt_index + 1,
                    endpoint_name: ep.name.clone(),
                    http_status: Some(status),
                    latency_ms,
                });
                ep_rt.failures.store(0, Ordering::SeqCst);
                let completed_at = Utc::now();
                // Sum attempt service times (excludes decode/hash/write and retry sleep).
                let logical_latency_ms: f64 = attempts.iter().map(|a| a.latency_ms).sum();
                return RequestOutcome {
                    request_id,
                    prompt_id: prompt.id,
                    prompt_sha256,
                    endpoint_name: ep.name,
                    endpoint_url: ep.url,
                    started_at,
                    completed_at,
                    latency_ms: logical_latency_ms,
                    first_byte_s: Some(first_byte.as_secs_f64()),
                    status: "success".to_string(),
                    http_status: Some(status),
                    n_requested: args.n,
                    n_returned,
                    size,
                    response_format: args.response_format.to_string(),
                    seed,
                    image_artifacts: artifacts,
                    response_bytes,
                    attempts,
                    error_type: None,
                    error_message: None,
                };
            }
            Err((err_type, http_status, err_message)) => {
                let latency_ms = attempt_start.elapsed().as_secs_f64() * 1000.0;
                attempts.push(AttemptRecord {
                    attempt: attempt_index + 1,
                    endpoint_name: ep.name.clone(),
                    http_status,
                    latency_ms,
                });
                ep_rt.failures.fetch_add(1, Ordering::SeqCst);
                if err_type == "connect" || err_type == "connection_error" {
                    if let Ok(mut guard) = ep_rt.ejected_until.lock() {
                        *guard = Some(Instant::now() + Duration::from_secs(5));
                    }
                }
                last_error_type = Some(err_type);
                last_error_message = Some(err_message);
                if attempt_index < args.endpoint_retry_attempts {
                    tokio::time::sleep(Duration::from_millis(args.endpoint_retry_backoff_ms)).await;
                }
            }
        }
    }

    let completed_at = Utc::now();
    let latency_ms: f64 = attempts.iter().map(|a| a.latency_ms).sum();
    let endpoint = attempts
        .last()
        .map(|a| a.endpoint_name.clone())
        .unwrap_or_else(|| "unknown".to_string());
    RequestOutcome {
        request_id,
        prompt_id: prompt.id,
        prompt_sha256,
        endpoint_name: endpoint.clone(),
        endpoint_url: endpoints
            .iter()
            .find(|e| e.name == endpoint)
            .map(|e| e.url.clone())
            .unwrap_or_default(),
        started_at,
        completed_at,
        latency_ms,
        first_byte_s: None,
        status: last_error_type
            .clone()
            .unwrap_or_else(|| "error".to_string()),
        http_status: attempts.last().and_then(|a| a.http_status),
        n_requested: args.n,
        n_returned: 0,
        size,
        response_format: args.response_format.to_string(),
        seed,
        image_artifacts: Vec::new(),
        response_bytes: 0,
        attempts,
        error_type: last_error_type,
        error_message: last_error_message,
    }
}

fn choose_endpoint(
    strategy: LoadBalancer,
    endpoints: &[Endpoint],
    runtime: &HashMap<String, EndpointRuntime>,
    rr: &AtomicUsize,
    random_index: usize,
) -> Endpoint {
    let is_ejected = |name: &str| -> bool {
        runtime
            .get(name)
            .and_then(|rt| {
                rt.ejected_until
                    .lock()
                    .ok()
                    .and_then(|g| (*g).map(|until| Instant::now() < until))
            })
            .unwrap_or(false)
    };
    match strategy {
        LoadBalancer::LeastInflight => endpoints
            .iter()
            .min_by_key(|ep| {
                let ejected = is_ejected(&ep.name);
                let inflight = runtime
                    .get(&ep.name)
                    .map(|rt| rt.inflight.load(Ordering::SeqCst))
                    .unwrap_or(usize::MAX);
                (u8::from(ejected), inflight)
            })
            .unwrap()
            .clone(),
        LoadBalancer::Random => {
            let idx = random_index % endpoints.len();
            endpoints[idx].clone()
        }
        LoadBalancer::WeightedRoundRobin => {
            let weighted: Vec<&Endpoint> = endpoints
                .iter()
                .flat_map(|ep| std::iter::repeat_n(ep, ep.weight as usize))
                .collect();
            let idx = rr.fetch_add(1, Ordering::SeqCst) % weighted.len();
            weighted[idx].clone()
        }
        LoadBalancer::RoundRobin => {
            let idx = rr.fetch_add(1, Ordering::SeqCst) % endpoints.len();
            endpoints[idx].clone()
        }
    }
}

fn resolve_seed(args: &Args, prompt: &PromptRow, request_index: usize) -> Option<i64> {
    match args.seed_mode {
        SeedMode::Prompt => prompt.seed.or(args.seed),
        SeedMode::Fixed => prompt.seed.or(args.seed),
        SeedMode::Increment => prompt
            .seed
            .or_else(|| args.seed.map(|s| s + request_index as i64)),
    }
}

async fn make_image_request(
    request_index: usize,
    attempt_index: usize,
    args: &Args,
    client: &Client,
    endpoint: &Endpoint,
    prompt: &PromptRow,
    seed: Option<i64>,
    size: &str,
) -> Result<(u16, Vec<ImageArtifact>, usize, u32, Duration, Duration), (String, Option<u16>, String)>
{
    let url = generations_url(&endpoint.url);
    let mut body = Map::new();
    body.insert("model".to_string(), json!(args.model));
    body.insert("prompt".to_string(), json!(prompt.prompt));
    body.insert("n".to_string(), json!(args.n));
    body.insert("size".to_string(), json!(size));
    body.insert(
        "response_format".to_string(),
        json!(args.response_format.to_string()),
    );
    if let Some(seed) = seed {
        body.insert("seed".to_string(), json!(seed));
    }
    if let Some(v) = prompt
        .negative_prompt
        .as_ref()
        .or(args.negative_prompt.as_ref())
    {
        body.insert("negative_prompt".to_string(), json!(v));
    }
    if let Some(v) = args.num_inference_steps {
        body.insert("num_inference_steps".to_string(), json!(v));
    }
    if let Some(v) = args.guidance_scale {
        body.insert("guidance_scale".to_string(), json!(v));
    }
    if let Some(v) = args.true_cfg_scale {
        body.insert("true_cfg_scale".to_string(), json!(v));
    }
    if let Some(extra) =
        load_extra_body(args).map_err(|e| ("schema_error".to_string(), None, e.to_string()))?
    {
        for (k, v) in extra {
            body.insert(k, v);
        }
    }

    let send_start = Instant::now();
    let resp = client
        .post(&url)
        .bearer_auth(&endpoint.api_key)
        .json(&Value::Object(body))
        .timeout(Duration::from_secs(args.request_timeout))
        .send()
        .await
        .map_err(|e| {
            let typed = metrum_ai_bench::error::RequestError::from_reqwest(&e);
            let err_type = match typed {
                metrum_ai_bench::error::RequestError::Timeout => "timeout",
                metrum_ai_bench::error::RequestError::Connect => "connect",
                _ => "request_error",
            };
            (
                err_type.to_string(),
                e.status().map(|s| s.as_u16()),
                e.to_string(),
            )
        })?;
    let first_byte = send_start.elapsed();
    let status = resp.status().as_u16();
    let bytes = resp.bytes().await.map_err(|e| {
        let typed = metrum_ai_bench::error::RequestError::from_reqwest(&e);
        let err_type = match typed {
            metrum_ai_bench::error::RequestError::Connect => "connect",
            metrum_ai_bench::error::RequestError::Timeout => "timeout",
            _ => "connection_error",
        };
        (err_type.to_string(), Some(status), e.to_string())
    })?;
    // Service latency ends once the response body is fully read; decode/hash/write
    // below are client-side and excluded from the measured interval.
    let service_latency = send_start.elapsed();
    if !(200..300).contains(&status) {
        return Err((
            "http_error".to_string(),
            Some(status),
            String::from_utf8_lossy(&bytes).to_string(),
        ));
    }
    let response_bytes = bytes.len();
    let parsed: Value = serde_json::from_slice(&bytes)
        .map_err(|e| ("schema_error".to_string(), Some(status), e.to_string()))?;
    if args.save_response_json {
        let response_path = Path::new(&args.artifact_dir)
            .join("responses")
            .join(format!(
                "{:06}-attempt-{}.json",
                request_index + 1,
                attempt_index + 1
            ));
        fs::write(
            response_path,
            serde_json::to_vec_pretty(&parsed).unwrap_or_default(),
        )
        .map_err(|e| ("artifact_error".to_string(), Some(status), e.to_string()))?;
    }
    let data = parsed
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            (
                "schema_error".to_string(),
                Some(status),
                "response missing data array".to_string(),
            )
        })?;
    let mut artifacts = Vec::new();
    if args.response_format == ResponseFormat::B64Json {
        for (idx, item) in data.iter().enumerate() {
            let b64 = item
                .get("b64_json")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    (
                        "schema_error".to_string(),
                        Some(status),
                        "image item missing b64_json".to_string(),
                    )
                })?;
            let image_bytes = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|e| ("decode_error".to_string(), Some(status), e.to_string()))?;
            let img = image::load_from_memory(&image_bytes)
                .map_err(|e| ("decode_error".to_string(), Some(status), e.to_string()))?;
            let (width, height) = img.dimensions();
            let sha = hex_sha256(&image_bytes);
            let path =
                Path::new(&args.artifact_dir).join(format!("{:06}-{}.png", request_index + 1, idx));
            if !args.no_save_images {
                fs::write(&path, &image_bytes)
                    .map_err(|e| ("artifact_error".to_string(), Some(status), e.to_string()))?;
            }
            artifacts.push(ImageArtifact {
                index: idx,
                path: if args.no_save_images {
                    String::new()
                } else {
                    path.to_string_lossy().to_string()
                },
                sha256: sha,
                bytes: image_bytes.len(),
                mime_type: "image/png".to_string(),
                width,
                height,
            });
        }
    }
    Ok((
        status,
        artifacts,
        response_bytes,
        data.len() as u32,
        first_byte,
        service_latency,
    ))
}

fn load_extra_body(
    args: &Args,
) -> Result<Option<Map<String, Value>>, Box<dyn Error + Send + Sync>> {
    let raw = if let Some(s) = &args.extra_body_json {
        Some(s.clone())
    } else if let Some(path) = &args.extra_body_file {
        Some(fs::read_to_string(path)?)
    } else {
        None
    };
    if let Some(raw) = raw {
        let v: Value = serde_json::from_str(&raw)?;
        let obj = v
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("extra body must be a JSON object"))?;
        return Ok(Some(obj.clone()));
    }
    Ok(None)
}

async fn write_error(
    error_log: &Arc<Mutex<File>>,
    line: &str,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut file = error_log.lock().await;
    writeln!(file, "{}", line)?;
    Ok(())
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{:02x}", b)).collect()
}

fn parse_size(size: &str) -> Option<(u32, u32)> {
    let (w, h) = size.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_size() {
        assert_eq!(parse_size("1024x512"), Some((1024, 512)));
        assert_eq!(parse_size("bad"), None);
    }

    #[test]
    fn generations_url_accepts_base_or_full_path() {
        assert_eq!(
            generations_url("http://localhost:8000/v1"),
            "http://localhost:8000/v1/images/generations"
        );
        assert_eq!(
            generations_url("http://localhost:8000/v1/"),
            "http://localhost:8000/v1/images/generations"
        );
        assert_eq!(
            generations_url("http://localhost:8000/v1/images/generations"),
            "http://localhost:8000/v1/images/generations"
        );
        assert_eq!(
            generations_url("http://localhost:8000/v1/images/generations/"),
            "http://localhost:8000/v1/images/generations"
        );
    }
}
