// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0
#![allow(clippy::too_many_arguments)]

use chrono::Utc;
use clap::Parser;
use futures_util::StreamExt;
use log::{debug, error, info, trace, warn};
use metrum_ai_bench::endpoints::resolve_endpoints;
use metrum_ai_bench::prompt_inputs::load_metrum_ai_bench_llm_prompts;
use metrum_ai_bench::unique_id;
use rand::seq::SliceRandom;
use rand::SeedableRng;
use reqwest::{Client, Response};
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

fn effective_ramp_up_seconds(ramp_up_seconds: Option<u64>) -> Option<u64> {
    ramp_up_seconds.filter(|seconds| *seconds > 0)
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum MetrumAiBenchLLMMode {
    Chat,
    Completion,
}

impl std::fmt::Display for MetrumAiBenchLLMMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MetrumAiBenchLLMMode::Chat => write!(f, "chat"),
            MetrumAiBenchLLMMode::Completion => write!(f, "completion"),
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
        help = "URL of the AI model endpoint (use --endpoints-file for multiple)"
    )]
    url: Option<String>,

    #[arg(
        long,
        help = "Path to endpoints config file (YAML, curl-style). Mutually exclusive with --url/--api-key"
    )]
    endpoints_file: Option<String>,

    #[arg(long, value_parser = clap::value_parser!(u32).range(1..), help = "Number of requests to send (must be >= 1)")]
    num_requests: u32,

    #[arg(long, value_parser = clap::value_parser!(u32).range(1..), help = "Number of concurrent requests (must be >= 1)")]
    concurrency: u32,

    #[arg(
        long,
        help = "Path to a JSONL file or http(s) URL of JSONL prompts (one JSON object per line with \"prompt\" field)"
    )]
    prompts: String,

    #[arg(long, value_enum, help = "Mode of operation: 'chat' or 'completion'")]
    mode: MetrumAiBenchLLMMode,

    #[arg(long, help = "Enable streaming mode")]
    streaming: bool,

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

    #[arg(long, help = "Maximum number of tokens")]
    max_tokens: u32,

    #[arg(long, default_value_t = 0.1, help = "Temperature for sampling")]
    temperature: f32,

    #[arg(long, default_value = "debug.log", help = "Path to the debug log file")]
    debug_log: String,

    #[arg(long, default_value = "error.log", help = "Path to the error log file")]
    error_log: String,

    #[arg(long, default_value = "300", help = "Request timeout in seconds")]
    request_timeout: u64,

    #[arg(long, default_value = "30", help = "Connect timeout in seconds")]
    connect_timeout: u64,

    #[arg(long, default_value = "60", help = "Pool idle timeout in seconds")]
    pool_idle_timeout: u64,

    #[arg(long, default_value = "60", help = "TCP keepalive in seconds")]
    tcp_keepalive: u64,

    #[arg(
        long,
        help = "API key for authentication (use --endpoints-file for multiple)"
    )]
    api_key: Option<String>,

    #[arg(long, help = "Stop sending new requests after N seconds")]
    stop_after_seconds: Option<u64>,

    #[arg(
        long,
        help = "Ramp up period in seconds to gradually increase concurrency"
    )]
    ramp_up_seconds: Option<u64>,
}

/// Makes a request to the AI model endpoint and measures various performance metrics.
///
/// # Arguments
/// * `client` - HTTP client for making requests
/// * `url` - Endpoint URL
/// * `payload` - JSON payload for the request
/// * `streaming` - Whether to use streaming mode
/// * `request_timeout` - Request timeout in seconds
/// * `api_key` - API key for authentication
///
struct StreamMetrics {
    latency: Duration,
    first_byte: Duration,
    /// `None` for non-streaming responses (TTFT is undefined; do not fabricate).
    ttft: Option<Duration>,
    first_reasoning: Option<Duration>,
    itl: Vec<Duration>,
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    prompt_words: usize,
    completion_words: usize,
    prompt_text: String,
    completion_text: String,
}

async fn make_request(
    client: &Client,
    url: &str,
    payload: Value,
    mode: MetrumAiBenchLLMMode,
    streaming: bool,
    request_timeout: u64,
    api_key: &str,
) -> Result<StreamMetrics, Box<dyn Error + Send + Sync>> {
    let start_time = Instant::now();

    // Get prompt text for word counting (chat: messages[].content, completion: prompt)
    let prompt = match mode {
        MetrumAiBenchLLMMode::Chat => payload["messages"]
            .as_array()
            .and_then(|msgs| msgs.last())
            .and_then(|msg| msg["content"].as_str())
            .unwrap_or(""),
        MetrumAiBenchLLMMode::Completion => payload["prompt"].as_str().unwrap_or(""),
    };
    let prompt_word_count = count_words(prompt);

    // Log request details in debug mode
    debug!("Making HTTP request:");
    debug!("URL: {}", url);
    debug!("Headers:");
    debug!("  Content-Type: application/json");
    debug!("  Authorization: Bearer {}", "*".repeat(8)); // Don't log the actual API key
    debug!(
        "Request Body: {}",
        serde_json::to_string_pretty(&payload).unwrap_or_default()
    );

    if streaming {
        // Handle streaming response
        let response = match client
            .post(url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&payload)
            .timeout(Duration::from_secs(request_timeout))
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                log_reqwest_error_details(&e, "HTTP Request Failed");
                error!("Request context:");
                error!("  URL: {}", url);
                error!(
                    "  Payload: {}",
                    serde_json::to_string_pretty(&payload).unwrap_or_default()
                );
                error!("  Timeout: {}s", request_timeout);
                error!("  API Key Length: {} chars", api_key.len());
                error!("  Streaming Mode: {}", streaming);
                return Err(metrum_ai_bench::error::RequestError::from_reqwest(&e).into());
            }
        };
        let first_byte = start_time.elapsed();

        // Check for HTTP errors
        if !response.status().is_success() {
            let status = response.status();
            let headers = response.headers().clone();
            let error_body = response
                .text()
                .await
                .unwrap_or_else(|_| "No error body".to_string());

            error!(
                "Request failed with status: {} - Body: {}",
                status, error_body
            );
            error!("Request context:");
            error!("  URL: {}", url);
            error!(
                "  Payload: {}",
                serde_json::to_string_pretty(&payload).unwrap_or_default()
            );
            error!("  Headers: {:?}", headers);
            return Err(metrum_ai_bench::error::RequestError::from_status(status.as_u16()).into());
        }

        trace!("Request started streaming");
        let mut stream = response.bytes_stream();
        let mut parser = metrum_ai_bench::sse::SseParser::new();
        let mut first_token_time = None;
        let mut first_reasoning_time = None;
        let mut last_token_at: Option<Instant> = None;
        let mut itl: Vec<Duration> = Vec::new();
        let mut prompt_tokens = 0;
        let mut completion_tokens = 0;
        let mut total_tokens = 0;
        let mut done = false;
        let mut saw_finish = false;
        let mut completion_text = String::new();

        while let Some(item) = stream.next().await {
            let bytes = item.map_err(|e| metrum_ai_bench::error::RequestError::from_reqwest(&e))?;
            for event in parser.feed(&bytes) {
                match event {
                    metrum_ai_bench::sse::SseEvent::Done => {
                        done = true;
                    }
                    metrum_ai_bench::sse::SseEvent::Json(parsed) => {
                        if let Some(error) = parsed.get("error") {
                            return Err(metrum_ai_bench::error::RequestError::ApiError {
                                message: error.to_string(),
                            }
                            .into());
                        }
                        if let Some(usage) = parsed.get("usage") {
                            prompt_tokens = usage
                                .get("prompt_tokens")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(prompt_tokens);
                            completion_tokens = usage
                                .get("completion_tokens")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(completion_tokens);
                            total_tokens = usage
                                .get("total_tokens")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(total_tokens);
                        }
                        if let Some(choices) = parsed.get("choices").and_then(|c| c.as_array()) {
                            for choice in choices {
                                if metrum_ai_bench::sse::choice_finish_reason(choice).is_some() {
                                    saw_finish = true;
                                }
                                if first_reasoning_time.is_none()
                                    && metrum_ai_bench::sse::choice_has_reasoning_token(choice)
                                {
                                    first_reasoning_time = Some(start_time.elapsed());
                                }
                                if metrum_ai_bench::sse::choice_has_output_token(choice) {
                                    let now = Instant::now();
                                    if first_token_time.is_none() {
                                        first_token_time = Some(start_time.elapsed());
                                    } else if let Some(prev) = last_token_at {
                                        itl.push(now.saturating_duration_since(prev));
                                    }
                                    last_token_at = Some(now);
                                }
                                if let Some(content) =
                                    metrum_ai_bench::sse::choice_output_text(choice)
                                {
                                    completion_text.push_str(content);
                                }
                            }
                        }
                    }
                    metrum_ai_bench::sse::SseEvent::Raw(_) => {}
                }
            }
            if done {
                break;
            }
        }

        if !done && !saw_finish {
            return Err(metrum_ai_bench::error::RequestError::StreamTruncated.into());
        }
        let Some(ttft) = first_token_time else {
            return Err(metrum_ai_bench::error::RequestError::NoOutputToken.into());
        };
        trace!("Request completed successfully");
        let completion_word_count = count_words(&completion_text);
        Ok(StreamMetrics {
            latency: start_time.elapsed(),
            first_byte,
            ttft: Some(ttft),
            first_reasoning: first_reasoning_time,
            itl,
            prompt_tokens,
            completion_tokens,
            total_tokens,
            prompt_words: prompt_word_count,
            completion_words: completion_word_count,
            prompt_text: prompt.to_string(),
            completion_text,
        })
    } else {
        // Handle non-streaming response
        let response: Response = match client
            .post(url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&payload)
            .timeout(Duration::from_secs(request_timeout))
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => return Err(metrum_ai_bench::error::RequestError::from_reqwest(&e).into()),
        };
        let first_byte = start_time.elapsed();

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_else(|_| String::new());
            error!("HTTP {}: {}", status, error_body.trim());
            return Err(metrum_ai_bench::error::RequestError::from_status(status.as_u16()).into());
        }
        // Clock stops after the full body is consumed (not at headers).
        let json_resp: Value = response.json().await?;
        let total_time = start_time.elapsed();

        // Check for multiple choices in non-streaming response
        if let Some(choices) = json_resp.get("choices").and_then(|c| c.as_array()) {
            if choices.len() > 1 {
                warn!(
                    "Multiple choices ({}) returned by API endpoint - only processing first choice",
                    choices.len()
                );
            }
        }

        // Extract completion text: chat uses choices[0].message.content, completion uses choices[0].text
        let completion_text = match mode {
            MetrumAiBenchLLMMode::Chat => json_resp
                .get("choices")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("message"))
                .and_then(|m| m.get("content"))
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            MetrumAiBenchLLMMode::Completion => json_resp
                .get("choices")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or(""),
        };
        let completion_word_count = count_words(completion_text);

        // Extract token usage information. The request URL must be the full
        // chat-completions endpoint (e.g. .../v1/chat/completions); no path is appended.
        let prompt_tokens = json_resp
            .get("usage")
            .and_then(|usage| usage.get("prompt_tokens"))
            .map(parse_token_count)
            .unwrap_or(0);
        let completion_tokens = json_resp
            .get("usage")
            .and_then(|usage| usage.get("completion_tokens"))
            .map(parse_token_count)
            .unwrap_or(0);
        let total_tokens = json_resp
            .get("usage")
            .and_then(|usage| usage.get("total_tokens"))
            .map(parse_token_count)
            .unwrap_or(0);

        // Non-streaming: TTFT is not measured (do not fabricate latency as TTFT).
        Ok(StreamMetrics {
            latency: total_time,
            first_byte,
            ttft: None,
            first_reasoning: None,
            itl: Vec::new(),
            prompt_tokens,
            completion_tokens,
            total_tokens,
            prompt_words: prompt_word_count,
            completion_words: completion_word_count,
            prompt_text: prompt.to_string(),
            completion_text: completion_text.to_string(),
        })
    }
}

#[cfg(test)]
fn error_message_indicates_oom(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    if lower.contains("out of memory")
        || lower.contains("cuda oom")
        || lower.contains("oomkilled")
        || lower.contains("oom killed")
    {
        return true;
    }
    lower
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|token| token == "oom")
}

fn build_request_body(
    mode: MetrumAiBenchLLMMode,
    model: &str,
    max_tokens: u32,
    temperature: f32,
    prompt: &str,
    streaming: bool,
    ignore_eos: bool,
    min_tokens: Option<u32>,
    extra_body_json: Option<&str>,
    system_prompt: Option<&str>,
) -> Result<Value, Box<dyn Error + Send + Sync>> {
    let system = system_prompt.unwrap_or("You are a helpful assistant.");
    let mut base_payload = match mode {
        MetrumAiBenchLLMMode::Chat => {
            let mut messages = Vec::new();
            if !system.is_empty() {
                messages.push(json!({"role": "system", "content": system}));
            }
            messages.push(json!({"role": "user", "content": prompt}));
            json!({
                "model": model,
                "max_tokens": max_tokens,
                "temperature": temperature,
                "stream": streaming,
                "messages": messages
            })
        }
        MetrumAiBenchLLMMode::Completion => json!({
            "model": model,
            "max_tokens": max_tokens,
            "temperature": temperature,
            "stream": streaming,
            "prompt": prompt
        }),
    };

    if ignore_eos {
        base_payload["ignore_eos"] = json!(true);
    }
    if let Some(min_t) = min_tokens {
        base_payload["min_tokens"] = json!(min_t);
    }
    if streaming {
        if let Some(obj) = base_payload.as_object_mut() {
            obj.insert("stream_options".to_string(), json!({"include_usage": true}));
        }
    }
    if let Some(extra) = extra_body_json {
        let extra_val: Value = serde_json::from_str(extra)?;
        if let (Some(base), Some(extra_map)) = (base_payload.as_object_mut(), extra_val.as_object())
        {
            for (k, v) in extra_map {
                base.insert(k.clone(), v.clone());
            }
        }
    }
    Ok(base_payload)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    // Print banner
    metrum_ai_bench::banner::print_banner(VERSION, "metrum-ai-bench-llm");
    println!("metrum-ai-bench-llm version {}", VERSION);

    let args = Args::parse();

    // Check for version-only flag first
    if args.version_only {
        println!("metrum-ai-bench-llm version {}", VERSION);
        return Ok(());
    }

    let effective_ramp_up = effective_ramp_up_seconds(args.ramp_up_seconds);

    // Validate ramp-up period if specified and greater than zero.
    if let Some(ramp_up) = effective_ramp_up {
        if let Some(stop_after) = args.stop_after_seconds {
            if ramp_up >= stop_after {
                return Err(
                    anyhow::anyhow!("Ramp-up period must be less than stop-after period")
                        .context(format!(
                            "Ramp-up: {}s, Stop-after: {}s",
                            ramp_up, stop_after
                        ))
                        .context("Invalid configuration parameters")
                        .into(),
                );
            }
        }
    }

    // Resolve endpoints (single url+api_key or multi from file)
    let resolved_endpoints = resolve_endpoints(
        args.url.as_deref(),
        args.api_key.as_deref(),
        args.endpoints_file.as_deref(),
    )?;
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

    let mut prompts = load_metrum_ai_bench_llm_prompts(&args.prompts)?;

    // Shuffle prompts once for even distribution, then cycle through them deterministically
    let mut shuffle_rng = rand::rngs::StdRng::seed_from_u64(args.common.seed);
    prompts.shuffle(&mut shuffle_rng);
    let mut arrival_rng = rand::rngs::StdRng::seed_from_u64(args.common.seed.wrapping_add(1));
    let slots = metrum_ai_bench::load::schedule(
        args.common.arrival_kind(),
        args.num_requests as u64,
        args.common.request_rate.unwrap_or(0.0),
        &mut arrival_rng,
    );

    let sink = Arc::new(
        metrum_ai_bench::jsonl::JsonlSink::create(&args.data_log)
            .map_err(|e| format!("Failed to open data log file '{}': {}", args.data_log, e))?,
    );
    let stop = metrum_ai_bench::runner::StopFlag::new();
    metrum_ai_bench::runner::install_stop_handlers(stop.clone());
    let arrival_kind = args.common.arrival_kind();

    // Initialise semaphore with 1 permit if ramp-up is requested, otherwise with full capacity.
    let concurrency_limit = args.common.max_concurrency.unwrap_or(args.concurrency) as usize;
    let initial_permits: usize = if effective_ramp_up.is_some() {
        1
    } else {
        concurrency_limit
    };
    let semaphore = Arc::new(Semaphore::new(initial_permits));

    // Background task that gradually adds permits during the ramp-up window.
    if let Some(ramp_up) = effective_ramp_up {
        let semaphore_clone = semaphore.clone();
        let permits_to_add = concurrency_limit.saturating_sub(initial_permits);
        if permits_to_add > 0 {
            let interval_ms = (ramp_up as f64 / permits_to_add as f64 * 1000.0) as u64;
            tokio::spawn(async move {
                for _ in 0..permits_to_add {
                    tokio::time::sleep(Duration::from_millis(interval_ms)).await;
                    semaphore_clone.add_permits(1);
                }
            });
        }
    }

    let endpoint_selector = Arc::new(metrum_ai_bench::endpoints::EndpointSelector::new(
        &resolved_endpoints,
    ));

    let mut handles = vec![];
    let mut completed = 0;
    let mut last_percentage = 0;

    let start_time = Instant::now();
    let ramp_up_start = start_time;
    let (record_tx, mut record_rx) =
        tokio::sync::mpsc::unbounded_channel::<metrum_ai_bench::record::RequestRecord>();

    'request_loop: for slot in slots {
        if stop.is_stopped() {
            info!("Stop flag set; not issuing further requests");
            break 'request_loop;
        }
        if let Some(stop_after) = args.stop_after_seconds {
            if start_time.elapsed().as_secs() >= stop_after {
                info!(
                    "Reached time limit of {} seconds. Stopping new requests...",
                    stop_after
                );
                break 'request_loop;
            }
        }

        let wait = slot.scheduled_delay.saturating_sub(start_time.elapsed());
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }

        let i = slot.seq as usize;
        let phase = metrum_ai_bench::record::Phase::for_seq(slot.seq, args.common.warmup_requests);

        let permit = semaphore.clone().acquire_owned().await?;
        let ((url, api_key, endpoint_name), endpoint_lease) =
            endpoint_selector.select(&resolved_endpoints, args.common.load_balancer);
        let queue_delay = metrum_ai_bench::runner::queue_delay_for_slot(
            arrival_kind,
            start_time.elapsed(),
            slot.scheduled_delay,
        );
        let client = client.clone();
        let endpoint_selector = endpoint_selector.clone();
        let prompt = metrum_ai_bench::args_common::CommonBenchArgs::unique_prompt(
            &prompts[i % prompts.len()],
            slot.seq,
            args.common.unique_prompts,
            args.common.seed,
            &run_id,
        );

        let request_body = build_request_body(
            args.mode,
            &args.model,
            args.max_tokens,
            args.temperature,
            &prompt,
            args.streaming,
            args.common.ignore_eos,
            args.common.min_tokens,
            args.common.extra_body_json.as_deref(),
            args.common.system_prompt.as_deref(),
        )?;

        let request_timeout = args.request_timeout;
        let mode = args.mode;
        let streaming = args.streaming;
        let seq = slot.seq;
        let sink_task = sink.clone();
        let record_tx = record_tx.clone();
        let tokenizer_path = args.common.tokenizer.clone();
        let scheduled_delay = slot.scheduled_delay;
        let record_schedule = metrum_ai_bench::runner::should_record_schedule(arrival_kind);
        let run_id_task = run_id.clone();
        let run_start = start_time;
        let handle = tokio::spawn(async move {
            let send_offset = run_start.elapsed();
            let started_at = Utc::now();
            let send_instant = Instant::now();
            let result = make_request(
                &client,
                &url,
                request_body,
                mode,
                streaming,
                request_timeout,
                &api_key,
            )
            .await;
            drop(permit);
            drop(endpoint_lease);

            let tokenizer = match metrum_ai_bench::tokenizer::LocalTokenizer::from_file(
                tokenizer_path.as_deref(),
            ) {
                Ok(t) => t,
                Err(_) => metrum_ai_bench::tokenizer::LocalTokenizer::from_file(None)
                    .expect("disabled tokenizer"),
            };

            let record = match result {
                Ok(sm) => {
                    let completed_at =
                        metrum_ai_bench::runner::completed_at_from_start(started_at, sm.latency);
                    let tokenized_prompt_tokens = tokenizer.count(&sm.prompt_text).ok().flatten();
                    let tokenized_completion_tokens =
                        tokenizer.count(&sm.completion_text).ok().flatten();
                    let usage_missing = sm.completion_tokens == 0 && !sm.completion_text.is_empty();
                    let mut rec = metrum_ai_bench::record::RequestRecord::success(
                        seq,
                        phase,
                        endpoint_name.clone(),
                        started_at,
                        completed_at,
                        sm.latency,
                        sm.ttft,
                        sm.first_reasoning,
                        sm.itl,
                        sm.prompt_tokens,
                        sm.completion_tokens,
                        sm.total_tokens,
                    )
                    .with_first_byte(sm.first_byte)
                    .with_send_offset(send_offset);
                    if record_schedule {
                        rec = rec.with_schedule(scheduled_delay, queue_delay);
                    }
                    rec.tokenized_prompt_tokens = tokenized_prompt_tokens;
                    rec.tokenized_completion_tokens = tokenized_completion_tokens;
                    rec.usage_missing = usage_missing;
                    // Carry stream metrics fields via modality_metrics for console path.
                    rec.modality_metrics
                        .insert("prompt_words".into(), sm.prompt_words as f64);
                    rec.modality_metrics
                        .insert("completion_words".into(), sm.completion_words as f64);
                    rec.with_run_id(run_id_task)
                }
                Err(e) => {
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
    }
    drop(record_tx);

    info!("All requests launched. Waiting for completion...");

    let mut errors = 0;
    let mut metrics_started = false;
    let mut records: Vec<metrum_ai_bench::record::RequestRecord> = Vec::new();

    while let Some(rec) = record_rx.recv().await {
        let _endpoint_name = rec.endpoint.clone();
        let phase = rec.phase;
        if rec.is_success() {
            let response_time = Duration::from_secs_f64(rec.latency_s);
            let ttft = rec.ttft_s.map(Duration::from_secs_f64);
            let _prompt_tokens = rec.prompt_tokens;
            let completion_tokens = rec.completion_tokens;
            let total_tokens = rec.total_tokens;
            let _prompt_words = rec
                .modality_metrics
                .get("prompt_words")
                .copied()
                .unwrap_or(0.0) as usize;
            let _completion_words = rec
                .modality_metrics
                .get("completion_words")
                .copied()
                .unwrap_or(0.0) as usize;

            if let Some(ramp_up) = effective_ramp_up {
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
                    response_time.checked_sub(ttft).and_then(|gen_time| {
                        if gen_time.is_zero() {
                            None
                        } else {
                            Some(gen_time.as_secs_f64() / (completion_tokens - 1) as f64)
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

    // Ensure spawned tasks finished (channel already drained when all senders dropped).
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
    let window_seconds = metrum_ai_bench::runner::window_seconds_from_records(&records);
    let window_seconds = if window_seconds > 0.0 {
        window_seconds
    } else {
        // Fallback if no measured records produced a span.
        0.0
    };
    let slos = args.common.parse_slos()?;
    let body_template = build_request_body(
        args.mode,
        &args.model,
        args.max_tokens,
        args.temperature,
        "{{prompt}}",
        args.streaming,
        args.common.ignore_eos,
        args.common.min_tokens,
        args.common.extra_body_json.as_deref(),
        args.common.system_prompt.as_deref(),
    )?;
    let effective_system_prompt = args
        .common
        .effective_system_prompt("You are a helpful assistant.");
    let mut run_summary = metrum_ai_bench::summary::RunSummary::from_records_with_options(
        &records,
        window_seconds,
        stop.is_stopped(),
        &slos,
        args.common.throughput_bin_seconds,
    )
    .with_config(metrum_ai_bench::summary::EffectiveRunConfig {
        run_id: run_id.clone(),
        common: (&args.common).into(),
        effective_max_concurrency: args.common.max_concurrency.unwrap_or(args.concurrency),
        effective_system_prompt,
        body_template,
        unique_prompt_nonce_template:
            metrum_ai_bench::args_common::CommonBenchArgs::unique_prompt_nonce_template(
                args.common.unique_prompts,
            ),
        modality: Default::default(),
    });
    run_summary.environment =
        metrum_ai_bench::environment::collect(ntp_offset_ms, Some(args.model.clone()));
    if let Err(e) = sink.write(&run_summary) {
        warn!("Failed to write summary JSONL: {e}");
    }
    run_summary.print_console();

    let measure_errors = records
        .iter()
        .filter(|r| r.phase == metrum_ai_bench::record::Phase::Measure && !r.is_success())
        .count();
    if args.common.fail_on_error && measure_errors > 0 {
        Err(anyhow::anyhow!("Test completed with errors")
            .context(format!("Total errors: {measure_errors}"))
            .context("Load test encountered failures")
            .into())
    } else {
        Ok(())
    }
}

// Add helper function for word counting if not already present
/// Comprehensive error logging function that extracts all available error details
fn log_detailed_error(e: &dyn std::error::Error, context: &str) {
    error!("=== DETAILED ERROR REPORT ===");
    error!("Context: {}", context);
    error!("Primary Error: {}", e);
    error!("Error Type: {}", std::any::type_name_of_val(e));

    // Log the full error chain
    let mut current_error = e;
    let mut error_level = 0;
    while let Some(source) = current_error.source() {
        error_level += 1;
        error!("  Caused by (level {}): {}", error_level, source);
        error!(
            "  Error Type (level {}): {}",
            error_level,
            std::any::type_name_of_val(source)
        );
        current_error = source;
    }

    // Try to extract additional details for common error types
    let error_string = e.to_string();
    if error_string.contains("timeout") {
        error!("Timeout Details: This appears to be a timeout error");
    }
    if error_string.contains("connection") {
        error!("Connection Details: This appears to be a connection-related error");
    }
    if error_string.contains("DNS") || error_string.contains("resolve") {
        error!("DNS Details: This appears to be a DNS resolution error");
    }
    if error_string.contains("TLS") || error_string.contains("SSL") {
        error!("TLS/SSL Details: This appears to be a TLS/SSL related error");
    }
    if error_string.contains("HTTP") {
        error!("HTTP Details: This appears to be an HTTP protocol error");
    }

    // Log the debug representation for maximum detail
    error!("Debug Representation: {:?}", e);
    error!("=== END ERROR REPORT ===");
}

/// Enhanced error logging for reqwest errors specifically
fn log_reqwest_error_details(e: &reqwest::Error, context: &str) {
    error!("=== REQWEST ERROR DETAILS ===");
    error!("Context: {}", context);
    error!("Error: {}", e);

    if e.is_timeout() {
        error!("Error Type: Timeout");
    }
    if e.is_connect() {
        error!("Error Type: Connection Error");
    }
    if e.is_body() {
        error!("Error Type: Body Error");
    }
    if e.is_decode() {
        error!("Error Type: Decode Error");
    }
    if e.is_redirect() {
        error!("Error Type: Redirect Error");
    }
    if e.is_request() {
        error!("Error Type: Request Error");
    }
    if let Some(url) = e.url() {
        error!("Failed URL: {}", url);
    }
    if let Some(status) = e.status() {
        error!("HTTP Status: {}", status);
    }

    // Log the full error chain
    log_detailed_error(e, "Reqwest Error Chain");
    error!("=== END REQWEST ERROR DETAILS ===");
}

fn count_words(text: &str) -> usize {
    text.split_whitespace().count()
}

#[allow(dead_code)]
fn stream_choice_output_text(choice: &serde_json::Value) -> Option<&str> {
    choice
        .get("delta")
        .and_then(|delta| delta.get("content"))
        .and_then(|content| content.as_str())
        .or_else(|| choice.get("text").and_then(|text| text.as_str()))
}

#[allow(dead_code)]
fn stream_choice_has_output_token(choice: &serde_json::Value) -> bool {
    if stream_choice_output_text(choice).is_some_and(|content| !content.is_empty()) {
        return true;
    }

    let Some(delta) = choice.get("delta") else {
        return false;
    };

    delta
        .get("function_call")
        .and_then(|function_call| function_call.get("arguments"))
        .and_then(|arguments| arguments.as_str())
        .is_some_and(|arguments| !arguments.is_empty())
        || delta
            .get("tool_calls")
            .and_then(|tool_calls| tool_calls.as_array())
            .is_some_and(|tool_calls| {
                tool_calls.iter().any(|tool_call| {
                    tool_call
                        .get("function")
                        .and_then(|function| function.get("arguments"))
                        .and_then(|arguments| arguments.as_str())
                        .is_some_and(|arguments| !arguments.is_empty())
                })
            })
}

/// Parse token count from JSON value; accepts u64 or i64 (clamped to 0).
fn parse_token_count(v: &serde_json::Value) -> u64 {
    v.as_u64()
        .or_else(|| v.as_i64().map(|n| n.max(0) as u64))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{
        effective_ramp_up_seconds, error_message_indicates_oom, stream_choice_has_output_token,
        stream_choice_output_text,
    };
    use serde_json::json;

    #[test]
    fn zero_ramp_up_is_treated_as_no_ramp_up() {
        assert_eq!(effective_ramp_up_seconds(None), None);
        assert_eq!(effective_ramp_up_seconds(Some(0)), None);
        assert_eq!(effective_ramp_up_seconds(Some(1)), Some(1));
    }

    #[test]
    fn oom_classifier_matches_explicit_oom_errors() {
        assert!(error_message_indicates_oom(
            "CUDA out of memory while allocating"
        ));
        assert!(error_message_indicates_oom("request failed: cuda oom"));
        assert!(error_message_indicates_oom("container was OOMKilled"));
        assert!(error_message_indicates_oom("worker reported OOM"));
    }

    #[test]
    fn oom_classifier_does_not_match_oom_substrings() {
        assert!(!error_message_indicates_oom(
            "model /weights/bloom-560m failed validation"
        ));
        assert!(!error_message_indicates_oom(
            "temporary room allocation failure"
        ));
    }

    #[test]
    fn stream_choice_output_text_ignores_role_only_chunks() {
        let choice = json!({"index": 0, "delta": {"role": "assistant"}});

        assert_eq!(stream_choice_output_text(&choice), None);
        assert!(!stream_choice_has_output_token(&choice));
    }

    #[test]
    fn stream_choice_output_text_preserves_empty_content() {
        let choice = json!({"index": 0, "delta": {"content": ""}});

        assert_eq!(stream_choice_output_text(&choice), Some(""));
        assert!(!stream_choice_has_output_token(&choice));
    }

    #[test]
    fn stream_choice_output_text_extracts_non_empty_delta_content() {
        let choice = json!({"index": 0, "delta": {"content": "hello"}});

        assert_eq!(stream_choice_output_text(&choice), Some("hello"));
        assert!(stream_choice_has_output_token(&choice));
    }

    #[test]
    fn stream_choice_output_text_supports_completion_text_chunks() {
        let choice = json!({"index": 0, "text": "hello"});

        assert_eq!(stream_choice_output_text(&choice), Some("hello"));
        assert!(stream_choice_has_output_token(&choice));
    }

    #[test]
    fn stream_choice_output_token_detects_function_call_arguments() {
        let choice = json!({
            "index": 0,
            "delta": {
                "function_call": {
                    "name": "lookup",
                    "arguments": "{\"city\""
                }
            }
        });

        assert_eq!(stream_choice_output_text(&choice), None);
        assert!(stream_choice_has_output_token(&choice));
    }

    #[test]
    fn stream_choice_output_token_detects_tool_call_arguments() {
        let choice = json!({
            "index": 0,
            "delta": {
                "tool_calls": [
                    {
                        "index": 0,
                        "id": "call_123",
                        "type": "function",
                        "function": {
                            "name": "lookup",
                            "arguments": "{\"city\""
                        }
                    }
                ]
            }
        });

        assert_eq!(stream_choice_output_text(&choice), None);
        assert!(stream_choice_has_output_token(&choice));
    }

    #[test]
    fn stream_choice_output_token_ignores_finish_chunks() {
        let choice = json!({"index": 0, "delta": {}, "finish_reason": "stop"});

        assert_eq!(stream_choice_output_text(&choice), None);
        assert!(!stream_choice_has_output_token(&choice));
    }
}
