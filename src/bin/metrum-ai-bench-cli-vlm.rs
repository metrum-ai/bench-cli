// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0
#![allow(clippy::too_many_arguments)]

use chrono::Utc;
use clap::Parser;
use log::{debug, error, info, warn};
use metrum_ai_bench::endpoints::resolve_endpoints;
use metrum_ai_bench::prompt_inputs::{is_http_url, load_metrum_ai_bench_vlm_records};
use metrum_ai_bench::unique_id;
use metrum_ai_bench::vlm::{build_request_body, ImageCache, ImageData};
use rand::seq::SliceRandom;
use rand::SeedableRng;
use reqwest::Client;
use serde_json::{json, Value};
use simplelog::*;
use std::fs::File;
use std::{
    error::Error,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Write a failed request.v3 for preprocess / body-build skips so attempted
/// counts and `--fail-on-error` stay honest (N-03).
fn emit_preprocess_failure(
    sink: &std::sync::Arc<metrum_ai_bench::jsonl::JsonlSink>,
    record_tx: &tokio::sync::mpsc::UnboundedSender<metrum_ai_bench::record::RequestRecord>,
    seq: u64,
    warmup_requests: u32,
    endpoint: &str,
    run_start: Instant,
    run_id: &str,
    arrival_kind: metrum_ai_bench::load::ArrivalKind,
    scheduled_delay: Duration,
    queue_delay: Duration,
    message: String,
) {
    let send_offset = run_start.elapsed();
    let started_at = Utc::now();
    let latency = Duration::ZERO;
    let phase = metrum_ai_bench::record::Phase::for_seq(seq, warmup_requests);
    let mut rec = metrum_ai_bench::record::RequestRecord::failed(
        seq,
        phase,
        endpoint.to_string(),
        started_at,
        metrum_ai_bench::runner::completed_at_from_start(started_at, latency),
        latency,
        metrum_ai_bench::error::RequestError::Other { message },
    )
    .with_send_offset(send_offset)
    .with_run_id(run_id);
    if metrum_ai_bench::runner::should_record_schedule(arrival_kind) {
        rec = rec.with_schedule(scheduled_delay, queue_delay);
    }
    if let Err(e) = sink.write(&rec) {
        warn!("Failed to write preprocess-failure JSONL: {e}");
    }
    let _ = record_tx.send(rec);
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum MetrumAiBenchVLMImageDetail {
    Low,
    High,
}

impl std::fmt::Display for MetrumAiBenchVLMImageDetail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MetrumAiBenchVLMImageDetail::Low => write!(f, "low"),
            MetrumAiBenchVLMImageDetail::High => write!(f, "high"),
        }
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about = "A tool for load testing AI models", long_about = None)]
struct Args {
    #[arg(long, help = "Print version information and exit")]
    version_only: bool,

    #[arg(
        long,
        default_value_t = false,
        help = "Opt-in NTP clock check; records offset when available (does not hard-fail)"
    )]
    ntp_check: bool,

    #[arg(long, help = "Descriptor for the scenario being run")]
    scenario: String,

    #[arg(
        long,
        requires = "api_key",
        help = "URL of the AI model endpoint (use --endpoints-file for multiple). Required with --api-key."
    )]
    url: Option<String>,

    #[arg(
        long,
        help = "Path to endpoints config file (YAML). Mutually exclusive with --url/--api-key"
    )]
    endpoints_file: Option<String>,

    #[arg(long, value_parser = clap::value_parser!(u32).range(1..), help = "Number of requests to send (must be >= 1)")]
    num_requests: u32,

    #[arg(long, value_parser = clap::value_parser!(u32).range(1..), help = "Number of concurrent requests (must be >= 1)")]
    concurrency: u32,

    #[arg(
        long,
        help = "Path to the JSONL file containing prompts (one object per line with \"prompt\" and \"image_urls\" or \"image_url\"; each image is a local path, file:// URI, http(s) URL, or base64 data: URL)"
    )]
    prompts: String,

    #[arg(
        long,
        default_value = "warn",
        help = "Log level: error, warn, info, debug, trace"
    )]
    log_level: String,

    #[arg(long, help = "Model identifier")]
    model: String,

    #[arg(long, help = "Path to the data log file")]
    data_log: String,

    #[command(flatten)]
    common: metrum_ai_bench::args_common::CommonBenchArgs,

    #[command(flatten)]
    telemetry: metrum_ai_bench::telemetry::TelemetryArgs,

    #[arg(
        long,
        default_value_t = false,
        help = "Enable streaming mode for measured TTFT/ITL"
    )]
    streaming: bool,

    #[arg(
        long,
        default_value_t = false,
        help = "When visible-token TTFT is missing, approximate it from HTTP time-to-first-byte and record provenance"
    )]
    infer_ttft_from_first_byte: bool,

    #[arg(long, help = "Maximum number of tokens")]
    max_tokens: u32,

    #[arg(long, default_value_t = 0.1, help = "Temperature for sampling")]
    temperature: f32,

    #[arg(long, default_value = "debug.log", help = "Path to the debug log file")]
    debug_log: String,

    #[arg(long, default_value = "error.log", help = "Path to the error log file")]
    error_log: String,

    #[arg(long, default_value = "120", help = "Request timeout in seconds")]
    request_timeout: u64,

    #[arg(long, default_value = "30", help = "Connect timeout in seconds")]
    connect_timeout: u64,

    #[arg(long, default_value = "60", help = "Pool idle timeout in seconds")]
    pool_idle_timeout: u64,

    #[arg(long, default_value = "60", help = "TCP keepalive in seconds")]
    tcp_keepalive: u64,

    #[arg(
        long,
        requires = "url",
        help = "API key sent as a Bearer token. Required with --url. Use any placeholder such as \"dummy\" for servers that do not check it. Use --endpoints-file for multiple endpoints."
    )]
    api_key: Option<String>,

    #[arg(long, help = "Stop sending new requests after N seconds")]
    stop_after_seconds: Option<u64>,

    #[arg(
        long,
        help = "Ramp up period in seconds to gradually increase concurrency"
    )]
    ramp_up_seconds: Option<u64>,

    #[arg(
        long,
        default_value = "1",
        help = "Number of images to include per request (OpenAI supports multiple images per prompt)"
    )]
    num_images_batch: usize,

    #[arg(long, default_value = "1000", value_parser = clap::value_parser!(u32).range(1..), help = "Size of the image cache (must be >= 1)")]
    image_cache_size: u32,

    #[arg(long, value_parser = clap::value_parser!(u32).range(1..), help = "Maximum image dimension (width/height) in pixels, must be >= 1 if set")]
    max_image_dimension: Option<u32>,

    #[arg(
        long,
        default_value_t = false,
        help = "Re-encode images as JPEG instead of sending the original bytes"
    )]
    reencode_jpeg: bool,

    #[arg(
        long,
        default_value = "low",
        value_enum,
        help = "Image detail level: 'low' or 'high'"
    )]
    image_detail: MetrumAiBenchVLMImageDetail,

    #[arg(
        long,
        default_value_t = false,
        help = "Send http(s) image URLs for the server to download instead of base64 encoding them; local paths, file:// and data: URLs are rejected in this mode"
    )]
    server_side_download: bool,
}

async fn make_request(
    client: &Client,
    url: &str,
    payload: Value,
    request_timeout: u64,
    api_key: &str,
    images: &[ImageData],
    streaming: bool,
    allow_missing_ttft: bool,
) -> Result<
    (
        Duration,
        Duration,
        Option<Duration>,
        u64,
        u64,
        u64,
        Vec<(u64, (u32, u32))>,
        Option<Duration>,
        Vec<Duration>,
        String,
        Option<u64>,
    ),
    Box<dyn Error + Send + Sync>,
> {
    let start_time = Instant::now();

    debug!("Request URL: {}", url);
    let payload_str = serde_json::to_string_pretty(&payload).unwrap_or_default();
    if payload_str.contains("base64") {
        debug!("Request payload: (redacted: contains image data)");
    } else {
        debug!("Request payload: {}", payload_str);
    }

    let response = match metrum_ai_bench::connect_timing::send(
        client
            .post(url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&payload)
            .timeout(Duration::from_secs(request_timeout)),
    )
    .await
    {
        Ok(resp) => resp,
        Err(e) => return Err(metrum_ai_bench::error::RequestError::from_reqwest(&e).into()),
    };
    let first_byte = start_time.elapsed();

    if !response.status().is_success() {
        let status = response.status();
        let headers = response.headers().clone();
        let error_body = response
            .text()
            .await
            .unwrap_or_else(|_| "No error body".to_string());
        warn!(
            "Request failed with status: {} - Body: {} (headers: {:?})",
            status, error_body, headers
        );
        return Err(metrum_ai_bench::error::RequestError::from_status(status.as_u16()).into());
    }

    let image_stats: Vec<(u64, (u32, u32))> = images
        .iter()
        .map(|img| (img.size_bytes, (img.width, img.height)))
        .collect();

    if streaming {
        let stream = metrum_ai_bench::chat_stream::consume_with_options(
            metrum_ai_bench::connect_timing::counted(response.bytes_stream()),
            start_time,
            allow_missing_ttft,
        )
        .await?;
        return Ok((
            stream.latency,
            first_byte,
            stream.ttft,
            stream.prompt_tokens,
            stream.completion_tokens,
            stream.total_tokens,
            image_stats,
            stream.first_reasoning,
            stream.itl,
            stream.completion_text,
            stream.reasoning_tokens,
        ));
    }

    let body = metrum_ai_bench::connect_timing::read_body(response).await?;
    let response_text = String::from_utf8_lossy(&body).into_owned();
    if response_text.contains("base64") {
        debug!("Response body: (redacted: contains image data)");
    } else {
        debug!("Response body: {}", response_text);
    }

    let json_resp: Value = serde_json::from_str(&response_text)?;
    let total_time = start_time.elapsed();

    let usage = json_resp.get("usage").ok_or("No usage information")?;
    let prompt_tokens = usage
        .get("prompt_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let completion_tokens = usage
        .get("completion_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let total_tokens = usage
        .get("total_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let reasoning_tokens = metrum_ai_bench::usage::reasoning_tokens(usage);
    let completion_text = json_resp
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Non-streaming: TTFT is not measured (do not fabricate latency/tokens).
    Ok((
        total_time,
        first_byte,
        None,
        prompt_tokens,
        completion_tokens,
        total_tokens,
        image_stats,
        None,
        Vec::new(),
        completion_text,
        reasoning_tokens,
    ))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let args = Args::parse();

    // Check for version-only flag first
    if args.version_only {
        println!("metrum-ai-bench-cli-vlm version {}", VERSION);
        return Ok(());
    }

    metrum_ai_bench::banner::print_banner(VERSION, "metrum-ai-bench-cli-vlm", args.common.quiet);

    metrum_ai_bench::measurement::ensure_warmup_leaves_measurement(
        u64::from(args.common.warmup_requests),
        u64::from(args.num_requests),
    )?;

    let (sut_block, redact_hostname) = args.common.resolve_sut()?;
    let telemetry_cfg = args.telemetry.resolve_config()?;

    // Resolve endpoints (single url+api_key or multi from file)
    let resolved_endpoints = resolve_endpoints(
        args.url.as_deref(),
        args.api_key.as_deref(),
        args.endpoints_file.as_deref(),
    )?;
    metrum_ai_bench::sut::warn_remote_benchmark_urls(resolved_endpoints.urls());

    // Validate ramp-up period if specified
    if let Some(ramp_up) = args.ramp_up_seconds {
        if let Some(stop_after) = args.stop_after_seconds {
            if ramp_up >= stop_after {
                return Err("Ramp-up period must be less than stop-after period".into());
            }
        }
    }
    let ntp_offset_ms = if args.ntp_check {
        let offset = metrum_ai_bench::timecheck::check_ntp_offset();
        if let Some(offset_ms) = offset {
            println!("NTP clock offset: {}ms", offset_ms);
        }
        offset
    } else {
        None
    };

    // Convert string log level to LevelFilter
    let log_level = match args.log_level.to_lowercase().as_str() {
        "error" => LevelFilter::Error,
        "warn" => LevelFilter::Warn,
        "info" => LevelFilter::Info,
        "debug" => LevelFilter::Debug,
        "trace" => LevelFilter::Trace,
        _ => LevelFilter::Warn,
    };

    // Initialize logging with configured level
    CombinedLogger::init(vec![
        WriteLogger::new(
            log_level,
            Config::default(),
            File::create(&args.debug_log).map_err(|e| {
                format!(
                    "Failed to create debug log file '{}': {}",
                    args.debug_log, e
                )
            })?,
        ),
        WriteLogger::new(
            LevelFilter::Error,
            Config::default(),
            File::create(&args.error_log).map_err(|e| {
                format!(
                    "Failed to create error log file '{}': {}",
                    args.error_log, e
                )
            })?,
        ),
        TermLogger::new(
            log_level,
            Config::default(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
    ])?;

    info!(
        "Starting load test with {} requests at {} concurrency",
        args.num_requests, args.concurrency
    );

    let run_id = unique_id::generate_uuid();

    let client = metrum_ai_bench::http_client::build_http_client(
        metrum_ai_bench::http_client::HttpClientOptions {
            request_timeout: Some(Duration::from_secs(args.request_timeout)),
            connect_timeout: Duration::from_secs(args.connect_timeout),
            pool_max_idle_per_host: args.concurrency as usize,
            pool_idle_timeout: Duration::from_secs(args.pool_idle_timeout),
            tcp_keepalive: Duration::from_secs(args.tcp_keepalive),
            ca_cert: args.common.ca_cert.as_deref().map(std::path::Path::new),
            insecure: args.common.insecure,
        },
    )?;

    let mut records = load_metrum_ai_bench_vlm_records(&args.prompts)?;
    records.shuffle(&mut rand::rngs::StdRng::seed_from_u64(args.common.seed));
    info!("Loaded {} prompts from '{}'", records.len(), args.prompts);

    let mut image_cache = ImageCache::new(args.image_cache_size as usize)?;
    if !args.server_side_download {
        info!("Preloading images into cache before the measurement window");
        for rec in &records {
            for url in &rec.1 {
                if let Err(e) = image_cache
                    .get_or_load(
                        &client,
                        url,
                        args.max_image_dimension,
                        args.request_timeout,
                        args.reencode_jpeg,
                    )
                    .await
                {
                    warn!(
                        "Image preload failed for {}: {e}",
                        metrum_ai_bench::prompt_inputs::image_ref_key(url)
                    );
                }
            }
        }
    }
    let concurrency_cap = args.common.max_concurrency.unwrap_or(args.concurrency);
    let semaphore = Arc::new(Semaphore::new(concurrency_cap as usize));
    let inflight_tracker = Arc::new(metrum_ai_bench::concurrency::InFlightTracker::new(
        concurrency_cap,
    ));
    let endpoint_selector = Arc::new(metrum_ai_bench::endpoints::EndpointSelector::new(
        &resolved_endpoints,
    ));
    let sink = Arc::new(metrum_ai_bench::jsonl::JsonlSink::create(&args.data_log)?);
    let stop = metrum_ai_bench::runner::StopFlag::new();
    metrum_ai_bench::runner::install_stop_handlers(stop.clone());
    let arrival_kind = args.common.arrival_kind();
    let mut handles = vec![];
    let mut completed = 0;
    let mut last_percentage = 0;
    let mut metrics_started = false;

    // Telemetry starts before the run clock so source probes are not timed.
    let mut telemetry = args
        .telemetry
        .start_session(
            telemetry_cfg.as_ref(),
            metrum_ai_bench::telemetry::RunStamp {
                run_id: run_id.clone(),
                tool_version: VERSION.to_string(),
                sut: sut_block.as_ref().map(serde_json::to_value).transpose()?,
                config: serde_json::json!({
                    "binary": "metrum-ai-bench-cli-vlm",
                    "scenario": args.scenario,
                    "model": args.model,
                    "streaming": args.streaming,
                    "num_requests": args.num_requests,
                    "concurrency": concurrency_cap,
                    "request_rate": args.common.request_rate,
                    "warmup_requests": args.common.warmup_requests,
                    "data_log": args.data_log,
                    "telemetry": args.telemetry.telemetry,
                }),
            },
            Some(stop.clone()),
        )
        .await?;
    if let Some(session) = telemetry.as_mut() {
        session.set_load(
            args.common
                .request_rate
                .unwrap_or(f64::from(concurrency_cap)),
        );
    }

    // With --ndjson the run clock is the NDJSON epoch, so data-log
    // send_offset_s / scheduled_offset_s equal t_sent_ns / t_sched_ns.
    let start_time = telemetry
        .as_ref()
        .map_or_else(Instant::now, |session| session.run_start());
    let ramp_up_start = start_time;
    let mut current_concurrency;
    let (record_tx, mut record_rx) =
        tokio::sync::mpsc::unbounded_channel::<metrum_ai_bench::record::RequestRecord>();

    let mut arrival_rng = rand::rngs::StdRng::seed_from_u64(args.common.seed.wrapping_add(1));
    let slots = metrum_ai_bench::load::schedule(
        arrival_kind,
        u64::from(args.num_requests),
        args.common.request_rate.unwrap_or(0.0),
        &mut arrival_rng,
    );
    'request_loop: for slot in slots {
        if stop.is_stopped() {
            info!("Stop flag set; not issuing further requests");
            break 'request_loop;
        }
        let i = slot.seq as usize;
        if let Some(stop_after) = args.stop_after_seconds {
            if start_time.elapsed().as_secs() >= stop_after {
                info!(
                    "Reached time limit of {} seconds. Stopping new requests...",
                    stop_after
                );
                break 'request_loop;
            }
        }

        // Handle ramp-up if specified
        if let Some(ramp_up) = args.ramp_up_seconds {
            let elapsed = ramp_up_start.elapsed().as_secs_f64();
            if elapsed < ramp_up as f64 {
                current_concurrency =
                    ((elapsed / ramp_up as f64) * (args.concurrency as f64)).ceil() as usize;
                if current_concurrency == 0 {
                    current_concurrency = 1;
                }
                if current_concurrency > (args.concurrency as usize) {
                    current_concurrency = args.concurrency as usize;
                }
                info!(
                    "Ramping up: current concurrency = {} (target = {})",
                    current_concurrency, args.concurrency
                );
            } else {
                current_concurrency = args.concurrency as usize;
            }
        } else {
            current_concurrency = args.concurrency as usize;
        }

        let wait = slot.scheduled_delay.saturating_sub(start_time.elapsed());
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        let permit = metrum_ai_bench::concurrency::acquire_with_engagement(
            semaphore.clone(),
            &inflight_tracker,
        )
        .await?;
        let queue_delay = metrum_ai_bench::runner::queue_delay_for_slot(
            arrival_kind,
            start_time.elapsed(),
            slot.scheduled_delay,
        );
        let ((url, api_key, endpoint_name), endpoint_lease) =
            endpoint_selector.select(&resolved_endpoints, args.common.load_balancer);
        let client_loop = client.clone();
        let endpoint_selector = endpoint_selector.clone();
        let selected_record = &records[i % records.len()];
        let mut selected_images = Vec::new();

        // server_side_download: only allow http(s) URLs; do not forward local paths or file://
        if args.server_side_download {
            let refs_to_check: Vec<&str> = if selected_record.1.len() > 1 {
                selected_record.1.iter().map(String::as_str).collect()
            } else if args.num_images_batch > 1 {
                let mut refs = vec![];
                if !selected_record.1.is_empty() {
                    refs.push(selected_record.1[0].as_str());
                }
                for _ in 1..args.num_images_batch {
                    if let Some(rec) = records.get((i + refs.len()) % records.len()) {
                        if !rec.1.is_empty() {
                            refs.push(rec.1[0].as_str());
                        }
                    }
                }
                refs
            } else if !selected_record.1.is_empty() {
                vec![selected_record.1[0].as_str()]
            } else {
                vec![]
            };
            if refs_to_check.iter().any(|r| !is_http_url(r)) {
                error!(
                    "server_side_download requires http(s) URLs; local paths, file:// and data: URLs are not sent (endpoint={})",
                    endpoint_name
                );
                emit_preprocess_failure(
                    &sink,
                    &record_tx,
                    slot.seq,
                    args.common.warmup_requests,
                    &endpoint_name,
                    start_time,
                    &run_id,
                    arrival_kind,
                    slot.scheduled_delay,
                    queue_delay,
                    "server_side_download requires http(s) image URLs".into(),
                );
                drop(permit);
                continue 'request_loop;
            }
        }

        // Build selected_images; on image load failure record error and skip this request
        let image_detail_str = format!("{}", args.image_detail);
        if selected_record.1.len() > 1 {
            for url in &selected_record.1 {
                if args.server_side_download {
                    selected_images.push(ImageData {
                        base64_data: String::new(),
                        mime_type: String::new(),
                        width: 0,
                        height: 0,
                        size_bytes: 0,
                        url: url.clone(),
                    });
                } else {
                    match image_cache
                        .get_or_load(
                            &client_loop,
                            url,
                            args.max_image_dimension,
                            args.request_timeout,
                            args.reencode_jpeg,
                        )
                        .await
                    {
                        Ok(image_data) => selected_images.push(image_data),
                        Err(e) => {
                            emit_preprocess_failure(
                                &sink,
                                &record_tx,
                                slot.seq,
                                args.common.warmup_requests,
                                &endpoint_name,
                                start_time,
                                &run_id,
                                arrival_kind,
                                slot.scheduled_delay,
                                queue_delay,
                                format!("image load failed: {e}"),
                            );
                            drop(permit);
                            continue 'request_loop;
                        }
                    }
                }
            }
        } else if args.num_images_batch > 1 {
            if !selected_record.1.is_empty() {
                if args.server_side_download {
                    selected_images.push(ImageData {
                        base64_data: String::new(),
                        mime_type: String::new(),
                        width: 0,
                        height: 0,
                        size_bytes: 0,
                        url: selected_record.1[0].clone(),
                    });
                } else {
                    match image_cache
                        .get_or_load(
                            &client_loop,
                            &selected_record.1[0],
                            args.max_image_dimension,
                            args.request_timeout,
                            args.reencode_jpeg,
                        )
                        .await
                    {
                        Ok(image_data) => selected_images.push(image_data),
                        Err(e) => {
                            emit_preprocess_failure(
                                &sink,
                                &record_tx,
                                slot.seq,
                                args.common.warmup_requests,
                                &endpoint_name,
                                start_time,
                                &run_id,
                                arrival_kind,
                                slot.scheduled_delay,
                                queue_delay,
                                format!("image load failed: {e}"),
                            );
                            drop(permit);
                            continue 'request_loop;
                        }
                    }
                }
            }
            for _ in 1..args.num_images_batch {
                let additional_record = &records[(i + selected_images.len()) % records.len()];
                if !additional_record.1.is_empty() {
                    if args.server_side_download {
                        selected_images.push(ImageData {
                            base64_data: String::new(),
                            mime_type: String::new(),
                            width: 0,
                            height: 0,
                            size_bytes: 0,
                            url: additional_record.1[0].clone(),
                        });
                    } else {
                        match image_cache
                            .get_or_load(
                                &client_loop,
                                &additional_record.1[0],
                                args.max_image_dimension,
                                args.request_timeout,
                                args.reencode_jpeg,
                            )
                            .await
                        {
                            Ok(image_data) => selected_images.push(image_data),
                            Err(e) => {
                                emit_preprocess_failure(
                                    &sink,
                                    &record_tx,
                                    slot.seq,
                                    args.common.warmup_requests,
                                    &endpoint_name,
                                    start_time,
                                    &run_id,
                                    arrival_kind,
                                    slot.scheduled_delay,
                                    queue_delay,
                                    format!("image load failed: {e}"),
                                );
                                drop(permit);
                                continue 'request_loop;
                            }
                        }
                    }
                }
            }
        } else if !selected_record.1.is_empty() {
            if args.server_side_download {
                selected_images.push(ImageData {
                    base64_data: String::new(),
                    mime_type: String::new(),
                    width: 0,
                    height: 0,
                    size_bytes: 0,
                    url: selected_record.1[0].clone(),
                });
            } else {
                match image_cache
                    .get_or_load(
                        &client_loop,
                        &selected_record.1[0],
                        args.max_image_dimension,
                        args.request_timeout,
                        args.reencode_jpeg,
                    )
                    .await
                {
                    Ok(image_data) => selected_images.push(image_data),
                    Err(e) => {
                        emit_preprocess_failure(
                            &sink,
                            &record_tx,
                            slot.seq,
                            args.common.warmup_requests,
                            &endpoint_name,
                            start_time,
                            &run_id,
                            arrival_kind,
                            slot.scheduled_delay,
                            queue_delay,
                            format!("image load failed: {e}"),
                        );
                        drop(permit);
                        continue 'request_loop;
                    }
                }
            }
        }

        let request_body = match build_request_body(
            &args.model,
            args.max_tokens,
            args.temperature,
            &metrum_ai_bench::args_common::CommonBenchArgs::unique_prompt(
                &selected_record.0,
                i as u64,
                args.common.unique_prompts,
                args.common.seed,
                &run_id,
            ),
            &selected_images,
            &image_detail_str,
            args.server_side_download,
            args.streaming,
            args.common.ignore_eos,
            args.common.min_tokens,
            args.common.extra_body_json.as_deref(),
            args.common.system_prompt.as_deref(),
        ) {
            Ok(b) => b,
            Err(e) => {
                emit_preprocess_failure(
                    &sink,
                    &record_tx,
                    slot.seq,
                    args.common.warmup_requests,
                    &endpoint_name,
                    start_time,
                    &run_id,
                    arrival_kind,
                    slot.scheduled_delay,
                    queue_delay,
                    format!("request body build failed: {e}"),
                );
                drop(permit);
                continue 'request_loop;
            }
        };

        let request_timeout = args.request_timeout;
        let streaming = args.streaming;
        let infer_ttft = args.infer_ttft_from_first_byte;
        let selected_images_clone = selected_images.clone();
        let phase = metrum_ai_bench::record::Phase::for_seq(slot.seq, args.common.warmup_requests);
        let seq = slot.seq;
        let scheduled_delay = slot.scheduled_delay;
        let record_schedule = metrum_ai_bench::runner::should_record_schedule(arrival_kind);
        let sink_task = sink.clone();
        let record_tx = record_tx.clone();
        let run_id_task = run_id.clone();
        let tokenizer_path = args.common.tokenizer.clone();
        let prompt_text = metrum_ai_bench::args_common::CommonBenchArgs::unique_prompt(
            &selected_record.0,
            i as u64,
            args.common.unique_prompts,
            args.common.seed,
            &run_id,
        );
        let run_start = start_time;
        let tracker_task = Arc::clone(&inflight_tracker);
        let handle = tokio::spawn(async move {
            let request_slot =
                metrum_ai_bench::concurrency::InFlightSlot::new(&tracker_task, permit);
            let in_flight_at_send = request_slot.in_flight();
            let send_offset = run_start.elapsed();
            let started_at = Utc::now();
            let send_instant = Instant::now();
            let connect_slot = metrum_ai_bench::connect_timing::ConnectSlot::new();
            let result = metrum_ai_bench::connect_timing::with_connect_slot(
                Arc::clone(&connect_slot),
                make_request(
                    &client_loop,
                    &url,
                    request_body,
                    request_timeout,
                    &api_key,
                    &selected_images_clone,
                    streaming,
                    infer_ttft,
                ),
            )
            .await;
            let http_trace = connect_slot.trace();
            // InFlightSlot leaves the gauge before freeing the permit (#189).
            drop(endpoint_lease);
            drop(request_slot);

            let tokenizer = match metrum_ai_bench::tokenizer::LocalTokenizer::from_file(
                tokenizer_path.as_deref(),
            ) {
                Ok(t) => t,
                Err(_) => metrum_ai_bench::tokenizer::LocalTokenizer::from_file(None)
                    .expect("disabled tokenizer"),
            };

            let record = match result {
                Ok((
                    response_time,
                    first_byte,
                    ttft,
                    prompt_tokens,
                    completion_tokens,
                    total_tokens,
                    image_stats,
                    first_reasoning,
                    itl,
                    completion_text,
                    reasoning_tokens,
                )) => {
                    let completed_at =
                        metrum_ai_bench::runner::completed_at_from_start(started_at, response_time);
                    let tokenized_prompt_tokens = tokenizer.count(&prompt_text).ok().flatten();
                    let tokenized_completion_tokens =
                        tokenizer.count(&completion_text).ok().flatten();
                    let usage_missing = completion_tokens == 0 && !completion_text.is_empty();
                    let resolved = metrum_ai_bench::measurement::resolve_ttft(
                        streaming,
                        ttft.map(|d| d.as_secs_f64()),
                        Some(first_byte.as_secs_f64()),
                        infer_ttft,
                    );
                    let mut rec = metrum_ai_bench::record::RequestRecord::success(
                        seq,
                        phase,
                        endpoint_name.clone(),
                        started_at,
                        completed_at,
                        response_time,
                        None,
                        first_reasoning,
                        itl,
                        prompt_tokens,
                        completion_tokens,
                        total_tokens,
                    )
                    .with_reasoning_tokens(reasoning_tokens)
                    .with_first_byte(first_byte)
                    .with_resolved_ttft(resolved)
                    .with_http_trace(http_trace)
                    .with_in_flight(in_flight_at_send)
                    .with_send_offset(send_offset);
                    if record_schedule {
                        rec = rec.with_schedule(scheduled_delay, queue_delay);
                    }
                    rec.tokenized_prompt_tokens = tokenized_prompt_tokens;
                    rec.tokenized_completion_tokens = tokenized_completion_tokens;
                    rec.usage_missing = usage_missing;
                    // Payload size is what the server actually received, so it
                    // reflects --max-image-dimension and --reencode-jpeg.
                    rec.modality_metrics
                        .insert("image_count".to_string(), image_stats.len() as f64);
                    rec.modality_metrics.insert(
                        "image_bytes".to_string(),
                        image_stats.iter().map(|(size, _)| *size).sum::<u64>() as f64,
                    );
                    // Carry image dims for console metrics via modality_metrics.
                    for (idx, (size, (w, h))) in image_stats.iter().enumerate() {
                        rec.modality_metrics
                            .insert(format!("image_{idx}_bytes"), *size as f64);
                        rec.modality_metrics
                            .insert(format!("image_{idx}_width"), f64::from(*w));
                        rec.modality_metrics
                            .insert(format!("image_{idx}_height"), f64::from(*h));
                    }
                    rec.with_run_id(run_id_task)
                }
                Err(e) => {
                    error!("Request failed: {:#}", e);
                    let mut source_opt = e.source();
                    while let Some(source) = source_opt {
                        error!("Caused by: {}", source);
                        source_opt = source.source();
                    }
                    if let Some(req_err) = e.downcast_ref::<reqwest::Error>() {
                        if let Some(status) = req_err.status() {
                            error!("HTTP Status: {}", status);
                        }
                        if let Some(url) = req_err.url() {
                            error!("URL: {}", url);
                        }
                        if req_err.is_timeout() {
                            error!("Timeout occurred during request");
                        }
                        if req_err.is_body() {
                            error!("Error reading body: {}", req_err);
                        }
                    }
                    let latency = send_instant.elapsed();
                    let completed_at =
                        metrum_ai_bench::runner::completed_at_from_start(started_at, latency);
                    let request_error =
                        metrum_ai_bench::error::RequestError::from_error(e.as_ref());
                    if matches!(request_error, metrum_ai_bench::error::RequestError::Connect) {
                        endpoint_selector.note_connect_failure(&endpoint_name);
                    }
                    let mut rec = metrum_ai_bench::record::RequestRecord::failed(
                        seq,
                        phase,
                        endpoint_name.clone(),
                        started_at,
                        completed_at,
                        latency,
                        request_error,
                    )
                    .with_http_trace(http_trace)
                    .with_in_flight(in_flight_at_send)
                    .with_send_offset(send_offset);
                    if record_schedule {
                        rec = rec.with_schedule(scheduled_delay, queue_delay);
                    }
                    rec.with_run_id(run_id_task)
                }
            };

            if let Err(e) = sink_task.write(&record) {
                warn!("Failed to write request JSONL: {e}");
            }
            let _ = record_tx.send(record);
        });
        handles.push(handle);

        debug!("Launching request {}", i + 1);

        if (i + 1) % 100 == 0 {
            info!("Launched {} requests...", i + 1);
        }

        // Add delay between requests during ramp-up to control concurrency
        if let Some(ramp_up) = args.ramp_up_seconds {
            if ramp_up_start.elapsed().as_secs_f64() < ramp_up as f64 {
                let delay = (ramp_up as f64 / current_concurrency as f64) * 1000.0;
                tokio::time::sleep(Duration::from_millis(delay as u64)).await;
            }
        }
    }
    drop(record_tx);

    info!("All requests launched. Waiting for completion...");

    let mut errors = 0;
    let mut records: Vec<metrum_ai_bench::record::RequestRecord> = Vec::new();
    while let Some(rec) = record_rx.recv().await {
        if let Some(session) = telemetry.as_mut() {
            session.record_request(&rec, start_time).await;
        }
        let _endpoint_name = rec.endpoint.clone();
        let phase = rec.phase;
        if rec.is_success() {
            let response_time = Duration::from_secs_f64(rec.latency_s);
            let ttft = rec.ttft_s.map(Duration::from_secs_f64);
            let _prompt_tokens = rec.prompt_tokens;
            let completion_tokens = rec.completion_tokens;
            let total_tokens = rec.total_tokens;
            let image_count = rec
                .modality_metrics
                .get("image_count")
                .copied()
                .unwrap_or(0.0) as usize;
            let mut image_stats = Vec::with_capacity(image_count);
            for idx in 0..image_count {
                let size = rec
                    .modality_metrics
                    .get(&format!("image_{idx}_bytes"))
                    .copied()
                    .unwrap_or(0.0) as u64;
                let w = rec
                    .modality_metrics
                    .get(&format!("image_{idx}_width"))
                    .copied()
                    .unwrap_or(0.0) as u32;
                let h = rec
                    .modality_metrics
                    .get(&format!("image_{idx}_height"))
                    .copied()
                    .unwrap_or(0.0) as u32;
                image_stats.push((size, (w, h)));
            }

            if let Some(ramp_up) = args.ramp_up_seconds {
                if !metrics_started && ramp_up_start.elapsed().as_secs() >= ramp_up {
                    metrics_started = true;
                    info!("Ramp-up complete. Starting metrics collection...");
                }
                if !metrics_started {
                    records.push(rec);
                    continue;
                }
            }
            if phase == metrum_ai_bench::record::Phase::Warmup {
                records.push(rec);
                continue;
            }

            let _tpot = match ttft {
                Some(ttft) if completion_tokens > 1 => {
                    response_time.checked_sub(ttft).and_then(|d| {
                        if d.is_zero() {
                            None
                        } else {
                            Some(d / (completion_tokens as u32 - 1))
                        }
                    })
                }
                _ => None,
            };
            completed += 1;
            debug!(
                "Request completed - RT: {:?}, TTFT: {:?}, Tokens: {}",
                response_time, ttft, total_tokens
            );
            let percentage = (completed * 100) / (args.num_requests as usize);
            if percentage >= last_percentage + 10 {
                info!(
                    "{}% complete ({}/{} requests)",
                    percentage, completed, args.num_requests
                );
                last_percentage = (percentage / 10) * 10;
            }
            if completed % 100 == 0 {
                info!("Completed {} requests...", completed);
            }
            records.push(rec);
        } else {
            let err_msg = rec
                .error
                .as_ref()
                .map(|e| e.to_string())
                .unwrap_or_else(|| "unknown".into());
            error!("Request failed: {err_msg}");
            errors += 1;
            records.push(rec);
        }
    }

    for handle in handles {
        if let Err(e) = handle.await {
            error!("Task join error: {e}");
            errors += 1;
        }
    }

    info!(
        "Completed {} out of {} requests ({} errors)",
        completed, args.num_requests, errors
    );
    let (telemetry_info, telemetry_verdict) =
        metrum_ai_bench::telemetry::close_session(telemetry, stop.is_stopped()).await;
    let window_seconds = metrum_ai_bench::runner::window_seconds_from_records(&records);
    let window_seconds = if window_seconds > 0.0 {
        window_seconds
    } else {
        start_time.elapsed().as_secs_f64()
    };
    let slos = args.common.parse_slos()?;
    let vlm_system = args
        .common
        .effective_system_prompt("You are a helpful assistant capable of understanding images.");
    let image_detail_str = format!("{}", args.image_detail);
    let mut body_template = build_request_body(
        &args.model,
        args.max_tokens,
        args.temperature,
        "{{prompt}}",
        &[],
        &image_detail_str,
        args.server_side_download,
        args.streaming,
        args.common.ignore_eos,
        args.common.min_tokens,
        args.common.extra_body_json.as_deref(),
        args.common.system_prompt.as_deref(),
    )?;
    if let Some(messages) = body_template["messages"].as_array_mut() {
        if let Some(user) = messages.iter_mut().find(|m| m["role"] == "user") {
            if let Some(content) = user["content"].as_array_mut() {
                content.push(json!({
                    "type": "image_url",
                    "image_url": {
                        "url": "<redacted>",
                        "detail": image_detail_str
                    }
                }));
            }
        }
    }
    let mut shared_summary = metrum_ai_bench::summary::RunSummary::from_records_with_options(
        &records,
        window_seconds,
        stop.is_stopped(),
        &slos,
        args.common.throughput_bin_seconds,
    )
    .with_config(metrum_ai_bench::summary::EffectiveRunConfig {
        run_id: run_id.clone(),
        common: metrum_ai_bench::args_common::EffectiveCommonArgs::from(&args.common)
            .with_scenario(Some(args.scenario.clone())),
        effective_max_concurrency: args.common.max_concurrency.unwrap_or(args.concurrency),
        effective_system_prompt: vlm_system,
        body_template,
        unique_prompt_nonce_template:
            metrum_ai_bench::args_common::CommonBenchArgs::unique_prompt_nonce_template(
                args.common.unique_prompts,
            ),
        modality: [
            (
                "reencode_jpeg".into(),
                serde_json::json!(args.reencode_jpeg),
            ),
            (
                "image_detail".into(),
                serde_json::json!(format!("{}", args.image_detail)),
            ),
            (
                "max_image_dimension".into(),
                serde_json::json!(args.max_image_dimension),
            ),
            (
                "server_side_download".into(),
                serde_json::json!(args.server_side_download),
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
    let price = metrum_ai_bench::summary::resolve_price_per_hour(
        args.common.price_per_hour,
        sut_block.as_ref(),
    );
    let measure_successes: Vec<_> = records
        .iter()
        .filter(|r| r.phase == metrum_ai_bench::record::Phase::Measure && r.is_success())
        .collect();
    let missing_ttft = measure_successes
        .iter()
        .filter(|r| r.ttft_s.is_none())
        .count();
    let approx_count = measure_successes
        .iter()
        .filter(|r| {
            matches!(
                r.ttft_source,
                Some(metrum_ai_bench::measurement::TtftSource::FirstByteApprox)
            )
        })
        .count();
    let no_output_token_errors = records
        .iter()
        .filter(|r| {
            r.phase == metrum_ai_bench::record::Phase::Measure
                && matches!(
                    r.error,
                    Some(metrum_ai_bench::error::RequestError::NoOutputToken)
                )
        })
        .count();
    let ttft_audit = metrum_ai_bench::measurement::audit_chat_ttft(
        args.streaming,
        args.infer_ttft_from_first_byte,
        measure_successes.len(),
        missing_ttft,
        approx_count,
        no_output_token_errors,
    )?;
    shared_summary = shared_summary
        .with_sut(sut_block)
        .with_telemetry(telemetry_info)
        .with_price(price)
        .with_observed_concurrency(Some(inflight_tracker.snapshot()))
        .with_ttft_audit(ttft_audit.approx_count, ttft_audit.warning);
    if let Err(e) = sink.write(&shared_summary) {
        warn!("Failed to write summary JSONL: {e}");
    }
    shared_summary.print_console();
    telemetry_verdict?;
    let measure_errors = records
        .iter()
        .filter(|r| r.phase == metrum_ai_bench::record::Phase::Measure && !r.is_success())
        .count();
    if args.common.fail_on_error && measure_errors > 0 {
        Err("Test completed with errors".into())
    } else {
        Ok(())
    }
}
