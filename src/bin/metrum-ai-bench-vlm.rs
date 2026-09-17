// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0
#![allow(clippy::too_many_arguments)]

use base64::Engine;
use chrono::Utc;
use clap::Parser;
use futures_util::StreamExt;
use log::{debug, error, info, warn};
use metrum_ai_bench::endpoints::resolve_endpoints;
use metrum_ai_bench::prompt_inputs::{is_http_url, load_metrum_ai_bench_vlm_records};
use metrum_ai_bench::unique_id;
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
        help = "Path to the JSONL file containing prompts (one object per line with \"prompt\" and \"image_urls\" or \"image_url\")"
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

    #[arg(
        long,
        default_value_t = false,
        help = "Enable streaming mode for measured TTFT/ITL"
    )]
    streaming: bool,

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
        help = "Whether to let the server download images instead of base64 encoding them"
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
        let mut stream = response.bytes_stream();
        let mut parser = metrum_ai_bench::sse::SseParser::new();
        let mut first_token_time = None;
        let mut first_reasoning_time = None;
        let mut previous_token_time = None;
        let mut itl = Vec::new();
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
                    metrum_ai_bench::sse::SseEvent::Done => done = true,
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
                                if first_token_time.is_none()
                                    && metrum_ai_bench::sse::choice_has_output_token(choice)
                                {
                                    first_token_time = Some(start_time.elapsed());
                                }
                                if metrum_ai_bench::sse::choice_has_reasoning_token(choice)
                                    && first_reasoning_time.is_none()
                                {
                                    first_reasoning_time = Some(start_time.elapsed());
                                }
                                if metrum_ai_bench::sse::choice_has_output_token(choice) {
                                    let now = start_time.elapsed();
                                    if let Some(previous) = previous_token_time {
                                        itl.push(now.saturating_sub(previous));
                                    }
                                    previous_token_time = Some(now);
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
        return Ok((
            start_time.elapsed(),
            first_byte,
            Some(ttft),
            prompt_tokens,
            completion_tokens,
            total_tokens,
            image_stats,
            first_reasoning_time,
            itl,
            completion_text,
        ));
    }

    let response_text = response.text().await?;
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
    ))
}

// Structure to hold image data and metadata
#[derive(Clone)]
struct ImageData {
    base64_data: String,
    mime_type: String,
    width: u32,
    height: u32,
    size_bytes: u64,
    url: String, // Add this field to store the original URL
}

/// Bounded LRU image cache (HashMap + VecDeque; no third-party lru crate).
struct ImageCache {
    map: std::collections::HashMap<String, ImageData>,
    order: std::collections::VecDeque<String>,
    capacity: usize,
}

impl ImageCache {
    fn new(capacity: usize) -> Result<Self, Box<dyn Error + Send + Sync>> {
        if capacity == 0 {
            return Err("image_cache_size must be >= 1".into());
        }
        Ok(Self {
            map: std::collections::HashMap::with_capacity(capacity),
            order: std::collections::VecDeque::with_capacity(capacity),
            capacity,
        })
    }

    fn get(&mut self, key: &str) -> Option<ImageData> {
        if !self.map.contains_key(key) {
            return None;
        }
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            let k = self.order.remove(pos).expect("index from position");
            self.order.push_back(k);
        }
        self.map.get(key).cloned()
    }

    fn put(&mut self, key: String, value: ImageData) {
        if self.map.contains_key(&key) {
            self.order.retain(|k| k != &key);
        } else if self.map.len() >= self.capacity {
            if let Some(evicted) = self.order.pop_front() {
                self.map.remove(&evicted);
            }
        }
        self.order.push_back(key.clone());
        self.map.insert(key, value);
    }

    async fn get_or_load(
        &mut self,
        client: &Client,
        path: &str,
        max_dimension: Option<u32>,
        timeout_secs: u64,
        reencode_jpeg: bool,
    ) -> Result<ImageData, Box<dyn Error + Send + Sync>> {
        if let Some(data) = self.get(path) {
            debug!("Cache hit for image: {}", path);
            return Ok(data);
        }

        debug!("Cache miss for image: {}", path);
        // Load and process image
        let image_data = if path.starts_with("http://") || path.starts_with("https://") {
            // For URLs, use client with timeout (no unbounded reqwest::get)
            debug!("Fetching image from URL: {}", path);
            let response = client
                .get(path)
                .timeout(Duration::from_secs(timeout_secs))
                .send()
                .await
                .map_err(|e| format!("Failed to fetch image from URL '{}': {}", path, e))?;
            if !response.status().is_success() {
                return Err(format!(
                    "Failed to fetch image from URL '{}': HTTP {}",
                    path,
                    response.status()
                )
                .into());
            }
            response
                .bytes()
                .await
                .map_err(|e| format!("Failed to read image data from URL '{}': {}", path, e))?
                .to_vec()
        } else {
            // For local files, use fs::read
            debug!("Loading image from local file: {}", path);
            tokio::fs::read(path)
                .await
                .map_err(|e| format!("Failed to read local image file '{}': {}", path, e))?
        };

        let detected_format = image::guess_format(&image_data).ok();
        let mut mime_type = match detected_format {
            Some(image::ImageFormat::Png) => "image/png",
            Some(image::ImageFormat::Gif) => "image/gif",
            Some(image::ImageFormat::WebP) => "image/webp",
            _ => "image/jpeg",
        }
        .to_string();

        // Read the header for dimensions; the payload stays byte-identical to
        // the source unless a resize or an explicit re-encode is requested.
        let (source_width, source_height) =
            image::ImageReader::new(std::io::Cursor::new(&image_data))
                .with_guessed_format()
                .map_err(|e| format!("Failed to read image header from '{}': {}", path, e))?
                .into_dimensions()
                .map_err(|e| format!("Failed to read image size from '{}': {}", path, e))?;

        let oversized =
            max_dimension.is_some_and(|max_dim| source_width > max_dim || source_height > max_dim);

        let (encoded, width, height) = if oversized || reencode_jpeg {
            let mut img = image::load_from_memory(&image_data)
                .map_err(|e| format!("Failed to decode image from '{}': {}", path, e))?;
            if oversized {
                let max_dim = max_dimension.expect("oversized implies a limit");
                let scale = max_dim as f32 / source_width.max(source_height) as f32;
                let new_width = (source_width as f32 * scale) as u32;
                let new_height = (source_height as f32 * scale) as u32;
                img = img.resize(new_width, new_height, image::imageops::FilterType::Lanczos3);
                debug!(
                    "Resized image from {}x{} to {}x{}",
                    source_width, source_height, new_width, new_height
                );
            }
            let format = if reencode_jpeg {
                image::ImageFormat::Jpeg
            } else {
                image::ImageFormat::Png
            };
            let mut cursor = std::io::Cursor::new(Vec::new());
            // JPEG cannot store alpha; drop it rather than failing the request.
            if format == image::ImageFormat::Jpeg {
                image::DynamicImage::ImageRgb8(img.to_rgb8()).write_to(&mut cursor, format)?;
            } else {
                img.write_to(&mut cursor, format)?;
            }
            mime_type = match format {
                image::ImageFormat::Jpeg => "image/jpeg",
                _ => "image/png",
            }
            .to_string();
            let dimensions = (img.width(), img.height());
            (cursor.into_inner(), dimensions.0, dimensions.1)
        } else {
            (image_data, source_width, source_height)
        };

        let base64_data = base64::engine::general_purpose::STANDARD.encode(&encoded);
        let size_bytes = encoded.len() as u64;

        let image_data = ImageData {
            base64_data,
            mime_type,
            width,
            height,
            size_bytes,
            url: path.to_string(), // Store the original URL/path
        };

        self.put(path.to_string(), image_data.clone());
        Ok(image_data)
    }
}

// Structure to hold image content options
#[derive(Debug, Clone)]
struct ImageContentOptions {
    detail: String,
    server_side_download: bool,
    // Add more options here as needed
    // format: String,
    // quality: u8,
    // etc.
}

impl Default for ImageContentOptions {
    fn default() -> Self {
        Self {
            detail: "low".to_string(),
            server_side_download: false,
        }
    }
}

// Function to format image content for the request
fn format_image_content(image: &ImageData, options: &ImageContentOptions) -> Value {
    let image_url = if options.server_side_download {
        // If server-side download is enabled, use the original URL
        // Note: This assumes the ImageData struct has a url field
        // We'll need to modify the ImageData struct to store the original URL
        image.url.clone()
    } else {
        // Otherwise, use base64 encoded data
        format!("data:{};base64,{}", image.mime_type, image.base64_data)
    };

    json!({
        "type": "image_url",
        "image_url": {
            "url": image_url,
            "detail": options.detail
        }
    })
}

fn build_request_body(
    model: &str,
    max_tokens: u32,
    temperature: f32,
    prompt: &str,
    images: &[ImageData],
    image_detail: &str,
    server_side_download: bool,
    streaming: bool,
    ignore_eos: bool,
    min_tokens: Option<u32>,
    extra_body_json: Option<&str>,
    system_prompt: Option<&str>,
) -> Result<Value, Box<dyn Error + Send + Sync>> {
    let system =
        system_prompt.unwrap_or("You are a helpful assistant capable of understanding images.");
    let mut messages = Vec::new();

    if !system.is_empty() {
        messages.push(json!({
            "role": "system",
            "content": system
        }));
    }

    let mut content = Vec::new();

    if !prompt.is_empty() {
        content.push(json!({
            "type": "text",
            "text": prompt
        }));
    }

    // Create image content options
    let image_options = ImageContentOptions {
        detail: image_detail.to_string(),
        server_side_download,
        // Add more options here as needed
    };

    // Format each image with the options
    for image in images {
        content.push(format_image_content(image, &image_options));
    }

    messages.push(json!({
        "role": "user",
        "content": content
    }));

    let mut body = json!({
        "model": model,
        "messages": messages,
        "max_tokens": max_tokens,
        "temperature": temperature,
        "stream": streaming
    });
    if ignore_eos {
        body["ignore_eos"] = json!(true);
    }
    if let Some(min_t) = min_tokens {
        body["min_tokens"] = json!(min_t);
    }
    if streaming {
        body["stream_options"] = json!({"include_usage": true});
    }
    if let Some(extra) = extra_body_json {
        let extra_val: Value = serde_json::from_str(extra)?;
        if let (Some(base), Some(extra_map)) = (body.as_object_mut(), extra_val.as_object()) {
            for (k, v) in extra_map {
                base.insert(k.clone(), v.clone());
            }
        }
    }

    let body_str = serde_json::to_string_pretty(&body).unwrap_or_default();
    if body_str.contains("base64") {
        debug!("Built request body: (redacted: contains image data)");
    } else {
        debug!("Built request body: {}", body_str);
    }
    Ok(body)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    // Print banner
    metrum_ai_bench::banner::print_banner(VERSION, "metrum-ai-bench-vlm");

    let args = Args::parse();

    // Check for version-only flag first
    if args.version_only {
        println!("metrum-ai-bench-vlm version {}", VERSION);
        return Ok(());
    }

    let (sut_block, redact_hostname) = args.common.resolve_sut()?;

    // Resolve endpoints (single url+api_key or multi from file)
    let resolved_endpoints = resolve_endpoints(
        args.url.as_deref(),
        args.api_key.as_deref(),
        args.endpoints_file.as_deref(),
    )?;

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
                    warn!("Image preload failed for {url}: {e}");
                }
            }
        }
    }
    let semaphore = Arc::new(Semaphore::new(
        args.common.max_concurrency.unwrap_or(args.concurrency) as usize,
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

    let start_time = Instant::now();
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
        let permit = semaphore.clone().acquire_owned().await?;
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
                    "server_side_download requires http(s) URLs; local paths and file:// are not sent (endpoint={})",
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
        let handle = tokio::spawn(async move {
            let send_offset = run_start.elapsed();
            let started_at = Utc::now();
            let send_instant = Instant::now();
            let result = make_request(
                &client_loop,
                &url,
                request_body,
                request_timeout,
                &api_key,
                &selected_images_clone,
                streaming,
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
                )) => {
                    let completed_at =
                        metrum_ai_bench::runner::completed_at_from_start(started_at, response_time);
                    let tokenized_prompt_tokens = tokenizer.count(&prompt_text).ok().flatten();
                    let tokenized_completion_tokens =
                        tokenizer.count(&completion_text).ok().flatten();
                    let usage_missing = completion_tokens == 0 && !completion_text.is_empty();
                    let mut rec = metrum_ai_bench::record::RequestRecord::success(
                        seq,
                        phase,
                        endpoint_name.clone(),
                        started_at,
                        completed_at,
                        response_time,
                        ttft,
                        first_reasoning,
                        itl,
                        prompt_tokens,
                        completion_tokens,
                        total_tokens,
                    )
                    .with_first_byte(first_byte)
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
        common: (&args.common).into(),
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
    shared_summary = shared_summary.with_sut(sut_block);
    if let Err(e) = sink.write(&shared_summary) {
        warn!("Failed to write summary JSONL: {e}");
    }
    shared_summary.print_console();
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
