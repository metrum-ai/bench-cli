// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0
#![allow(clippy::too_many_arguments)]

use base64::Engine;
use chrono::Utc;
use clap::Parser;
use futures_util::StreamExt;
use log::{debug, error, info, warn};
use lru::LruCache;
use metrumbench::compile_time_info;
use metrumbench::endpoints::{resolve_endpoints, ResolvedEndpoints};
use metrumbench::prompt_inputs::{is_http_url, load_metrumbench_vlm_records};
use metrumbench::unique_id;
use rand::seq::SliceRandom;
use rand::SeedableRng;
use reqwest::Client;
use serde_json::{json, Value};
use simplelog::*;
use std::fs::File;
use std::{
    collections::HashMap,
    error::Error,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum MetrumBenchVLMImageDetail {
    Low,
    High,
}

impl std::fmt::Display for MetrumBenchVLMImageDetail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MetrumBenchVLMImageDetail::Low => write!(f, "low"),
            MetrumBenchVLMImageDetail::High => write!(f, "high"),
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
    common: metrumbench::args_common::CommonBenchArgs,

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
    image_detail: MetrumBenchVLMImageDetail,

    #[arg(
        long,
        default_value_t = false,
        help = "Whether to let the server download images instead of base64 encoding them"
    )]
    server_side_download: bool,
}

#[derive(Default, Clone)]
struct EndpointMetrics {
    response_times: Vec<Duration>,
    ttft_times: Vec<Duration>,
    tpot_times: Vec<Duration>,
    prompt_tokens: Vec<u64>,
    completion_tokens: Vec<u64>,
    total_tokens: Vec<u64>,
    errors: Vec<String>,
    image_sizes: Vec<u64>,
    images_per_request: Vec<u64>,
    image_dimensions: Vec<(u32, u32)>,
}

struct Metrics {
    response_times: Vec<Duration>,
    ttft_times: Vec<Duration>,
    tpot_times: Vec<Duration>,
    prompt_tokens: Vec<u64>,
    completion_tokens: Vec<u64>,
    total_tokens: Vec<u64>,
    start_time: Instant,
    errors: Vec<String>,
    scenario: String,
    version: String,
    image_sizes: Vec<u64>,             // Track image sizes in bytes
    images_per_request: Vec<u64>,      // Track number of images per request
    image_dimensions: Vec<(u32, u32)>, // Track image dimensions (width, height)
    endpoint_metrics: HashMap<String, EndpointMetrics>,
}

impl Metrics {
    fn new(scenario: String) -> Self {
        Self {
            response_times: Vec::new(),
            ttft_times: Vec::new(),
            tpot_times: Vec::new(),
            prompt_tokens: Vec::new(),
            completion_tokens: Vec::new(),
            total_tokens: Vec::new(),
            start_time: Instant::now(),
            errors: Vec::new(),
            scenario,
            version: VERSION.to_string(),
            image_sizes: Vec::new(),
            images_per_request: Vec::new(),
            image_dimensions: Vec::new(),
            endpoint_metrics: HashMap::new(),
        }
    }

    fn record_success(
        &mut self,
        endpoint_name: &str,
        response_time: Duration,
        ttft: Option<Duration>,
        tpot: Option<Duration>,
        prompt_tokens: u64,
        completion_tokens: u64,
        total_tokens: u64,
        image_stats: &[(u64, (u32, u32))],
    ) {
        self.response_times.push(response_time);
        if let Some(ttft) = ttft {
            self.ttft_times.push(ttft);
        }
        if let Some(t) = tpot {
            self.tpot_times.push(t);
        }
        self.prompt_tokens.push(prompt_tokens);
        self.completion_tokens.push(completion_tokens);
        self.total_tokens.push(total_tokens);
        let images_in_request = image_stats.len() as u64;
        for (size, dims) in image_stats {
            self.image_sizes.push(*size);
            self.image_dimensions.push(*dims);
        }
        self.images_per_request.push(images_in_request);
        let ep = self
            .endpoint_metrics
            .entry(endpoint_name.to_string())
            .or_default();
        ep.response_times.push(response_time);
        if let Some(ttft) = ttft {
            ep.ttft_times.push(ttft);
        }
        if let Some(t) = tpot {
            ep.tpot_times.push(t);
        }
        ep.prompt_tokens.push(prompt_tokens);
        ep.completion_tokens.push(completion_tokens);
        ep.total_tokens.push(total_tokens);
        for (size, dims) in image_stats {
            ep.image_sizes.push(*size);
            ep.image_dimensions.push(*dims);
        }
        ep.images_per_request.push(images_in_request);
    }

    fn record_error(&mut self, endpoint_name: &str, error: String) {
        self.errors.push(error.clone());
        let ep = self
            .endpoint_metrics
            .entry(endpoint_name.to_string())
            .or_default();
        ep.errors.push(error);
    }

    fn calc_percentile(sorted_values: &[Duration], percentile: f64) -> Duration {
        if sorted_values.is_empty() {
            return Duration::default();
        }
        let index =
            ((sorted_values.len() as f64 * percentile / 100.0).ceil() as usize).saturating_sub(1);
        *sorted_values.get(index).unwrap_or(&Duration::default())
    }

    fn print_compact_block(
        &self,
        label: &str,
        response_times: &[Duration],
        ttft_times: &[Duration],
        prompt_tokens: &[u64],
        completion_tokens: &[u64],
        total_tokens: &[u64],
        errors: &[String],
        elapsed_secs: f64,
    ) {
        let requests = response_times.len() + errors.len();
        if elapsed_secs <= 0.0 {
            println!("\n=== {} ===\n  (no timing data)", label);
            return;
        }
        let req_rate = requests as f64 / elapsed_secs;
        let total_pt: u64 = prompt_tokens.iter().sum();
        let total_ct: u64 = completion_tokens.iter().sum();
        let total_t: u64 = total_tokens.iter().sum();
        let token_rate = total_t as f64 / elapsed_secs;
        println!("\n=== {} ===", label);
        println!("  Requests:    {}", requests);
        println!("  Errors:      {}", errors.len());
        if !response_times.is_empty() {
            let mut sorted_rt = response_times.to_vec();
            sorted_rt.sort();
            let avg_rt = sorted_rt.iter().sum::<Duration>() / sorted_rt.len() as u32;
            let p50 = Self::calc_percentile(&sorted_rt, 50.0);
            let p99 = Self::calc_percentile(&sorted_rt, 99.0);
            println!(
                "  Avg RT:      {:.3}s  (p50: {:.3}s, p99: {:.3}s)",
                avg_rt.as_secs_f64(),
                p50.as_secs_f64(),
                p99.as_secs_f64()
            );
        }
        if !ttft_times.is_empty() {
            let avg_ttft = ttft_times.iter().sum::<Duration>() / ttft_times.len() as u32;
            println!("  Avg TTFT:    {:.3}s", avg_ttft.as_secs_f64());
        }
        println!(
            "  Tokens:      {} prompt, {} completion",
            total_pt, total_ct
        );
        println!("  Token Rate:  {:.1} tokens/sec", token_rate);
        println!("  Req Rate:   {:.1} req/sec", req_rate);
    }

    fn print_stats(&self, resolved: &ResolvedEndpoints) {
        let elapsed_secs = self.start_time.elapsed().as_secs_f64();
        let names_with_weights = resolved.endpoint_names_for_display();
        if names_with_weights.len() > 1 {
            for (name, weight) in &names_with_weights {
                if let Some(ep) = self.endpoint_metrics.get(name) {
                    self.print_compact_block(
                        &format!("Endpoint: {} (weight: {})", name, weight),
                        &ep.response_times,
                        &ep.ttft_times,
                        &ep.prompt_tokens,
                        &ep.completion_tokens,
                        &ep.total_tokens,
                        &ep.errors,
                        elapsed_secs,
                    );
                }
            }
            self.print_compact_block(
                "AGGREGATE",
                &self.response_times,
                &self.ttft_times,
                &self.prompt_tokens,
                &self.completion_tokens,
                &self.total_tokens,
                &self.errors,
                elapsed_secs,
            );
            println!("\nScenario: {}", self.scenario);
            println!("Version: {}", self.version);
            return;
        }
        let calc_stats = |values: &[Duration]| {
            if values.is_empty() {
                return (
                    Duration::default(),
                    Duration::default(),
                    Duration::default(),
                    Duration::default(),
                    Duration::default(),
                    Duration::default(),
                    Duration::default(),
                );
            }
            let mut sorted = values.to_vec();
            sorted.sort();
            let len = sorted.len();
            let avg = sorted.iter().sum::<Duration>() / len.max(1) as u32;
            let p50 = Self::calc_percentile(&sorted, 50.0);
            let p90 = Self::calc_percentile(&sorted, 90.0);
            let p95 = Self::calc_percentile(&sorted, 95.0);
            let p99 = Self::calc_percentile(&sorted, 99.0);
            (
                *sorted.first().unwrap_or(&Duration::default()),
                *sorted.last().unwrap_or(&Duration::default()),
                avg,
                p50,
                p90,
                p95,
                p99,
            )
        };

        let calc_stats_u64 = |values: &[u64]| {
            if values.is_empty() {
                return (0, 0, 0.0, 0);
            }
            let mut sorted = values.to_vec();
            sorted.sort();
            let len = sorted.len();
            let avg = sorted.iter().sum::<u64>() as f64 / len.max(1) as f64;
            (
                *sorted.first().unwrap_or(&0),
                *sorted.last().unwrap_or(&0),
                avg,
                *sorted.get(len / 2).unwrap_or(&0),
            )
        };

        // Print response time stats only if we have successful responses
        if !self.response_times.is_empty() {
            let (rt_min, rt_max, rt_avg, rt_p50, rt_p90, rt_p95, rt_p99) =
                calc_stats(&self.response_times);
            println!("\nResponse Time Statistics:");
            println!("Min: {:?}, Max: {:?}, Avg: {:?}", rt_min, rt_max, rt_avg);
            println!(
                "p50: {:?}, p90: {:?}, p95: {:?}, p99: {:?}",
                rt_p50, rt_p90, rt_p95, rt_p99
            );

            let (ttft_min, ttft_max, ttft_avg, ttft_p50, ttft_p90, ttft_p95, ttft_p99) =
                calc_stats(&self.ttft_times);
            println!("\nTime to First Token Statistics:");
            println!(
                "Min: {:?}, Max: {:?}, Avg: {:?}",
                ttft_min, ttft_max, ttft_avg
            );
            println!(
                "p50: {:?}, p90: {:?}, p95: {:?}, p99: {:?}",
                ttft_p50, ttft_p90, ttft_p95, ttft_p99
            );

            let (pt_min, pt_max, pt_avg, pt_median) = calc_stats_u64(&self.prompt_tokens);
            let total_prompt_tokens: u64 = self.prompt_tokens.iter().sum();
            println!("\nPrompt Tokens Statistics:");
            println!(
                "Min: {}, Max: {}, Avg: {:.2}, Median: {}",
                pt_min, pt_max, pt_avg, pt_median
            );
            println!("Total Prompt Tokens: {}", total_prompt_tokens);

            let (ct_min, ct_max, ct_avg, ct_median) = calc_stats_u64(&self.completion_tokens);
            let total_completion_tokens: u64 = self.completion_tokens.iter().sum();
            println!("\nCompletion Tokens Statistics:");
            println!(
                "Min: {}, Max: {}, Avg: {:.2}, Median: {}",
                ct_min, ct_max, ct_avg, ct_median
            );
            println!("Total Completion Tokens: {}", total_completion_tokens);

            let (tt_min, tt_max, tt_avg, tt_median) = calc_stats_u64(&self.total_tokens);
            let total_all_tokens: u64 = self.total_tokens.iter().sum();
            println!("\nTotal Tokens Statistics:");
            println!(
                "Min: {}, Max: {}, Avg: {:.2}, Median: {}",
                tt_min, tt_max, tt_avg, tt_median
            );
            println!("Total Tokens Processed: {}", total_all_tokens);
        } else {
            println!("\nNo successful responses to calculate timing statistics.");
        }

        // Add TPOT statistics after TTFT statistics
        if !self.tpot_times.is_empty() {
            let (tpot_min, tpot_max, tpot_avg, tpot_p50, tpot_p90, tpot_p95, tpot_p99) =
                calc_stats(&self.tpot_times);
            println!("\nTime Per Output Token Statistics:");
            println!(
                "Min: {:?}, Max: {:?}, Avg: {:?}",
                tpot_min, tpot_max, tpot_avg
            );
            println!(
                "p50: {:?}, p90: {:?}, p95: {:?}, p99: {:?}",
                tpot_p50, tpot_p90, tpot_p95, tpot_p99
            );
        }

        // Add error statistics back
        println!("\nError Statistics:");
        println!("Total Errors: {}", self.errors.len());
        let error_rate = (self.errors.len() as f64
            / (self.response_times.len() + self.errors.len()) as f64)
            * 100.0;
        println!("Error Rate: {:.2}%", error_rate);

        let mut error_counts: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::new();
        for error in &self.errors {
            *error_counts.entry(error.as_str()).or_insert(0) += 1;
        }

        if !error_counts.is_empty() {
            println!("\nError Breakdown:");
            for (error_type, count) in error_counts.iter() {
                println!("  {} occurrences: {}", count, error_type);
            }
        }

        // Rest of the timing statistics...
        let total_time = self.start_time.elapsed();
        let requests = self.response_times.len();

        // Calculate rates only if we have a non-zero elapsed time
        let elapsed_secs = total_time.as_secs_f64();
        if elapsed_secs > 0.0 {
            let concurrent_requests = requests as f64 / elapsed_secs;
            let tokens_per_second = self.total_tokens.iter().sum::<u64>() as f64 / elapsed_secs;
            let prompt_tokens_per_second =
                self.prompt_tokens.iter().sum::<u64>() as f64 / elapsed_secs;
            let completion_tokens_per_second =
                self.completion_tokens.iter().sum::<u64>() as f64 / elapsed_secs;

            println!("\nTiming Statistics:");
            println!("Total Time: {:.2?}", total_time);
            if requests > 0 {
                println!(
                    "Average Request Latency: {:.2?}",
                    total_time / requests as u32
                );
            }
            println!("Average Requests/Second: {:.2}", concurrent_requests);
            println!("Prompt Tokens/Second: {:.2}", prompt_tokens_per_second);
            println!(
                "Completion Tokens/Second: {:.2}",
                completion_tokens_per_second
            );
            println!("Total Tokens/Second: {:.2}", tokens_per_second);
        } else {
            println!("\nTiming Statistics:");
            println!("Total Time: {:.2?}", total_time);
            println!("Not enough data to calculate rates (test duration too short)");
        }

        // Add detailed image statistics
        if !self.image_sizes.is_empty() {
            let (img_size_min, img_size_max, img_size_avg, img_size_median) =
                calc_stats_u64(&self.image_sizes);
            println!("\nImage Size Statistics (bytes):");
            println!(
                "Min: {}, Max: {}, Avg: {:.2}, Median: {}",
                img_size_min, img_size_max, img_size_avg, img_size_median
            );

            // Calculate and print dimension statistics
            let total_images = self.image_dimensions.len();
            let avg_width = self.image_dimensions.iter().map(|(w, _)| w).sum::<u32>() as f64
                / total_images as f64;
            let avg_height = self.image_dimensions.iter().map(|(_, h)| h).sum::<u32>() as f64
                / total_images as f64;

            println!("\nImage Dimension Statistics:");
            println!(
                "Average dimensions: {:.0}x{:.0} pixels",
                avg_width, avg_height
            );
            println!("Total Images Processed: {}", total_images);

            // Calculate and print multi-image statistics if we have any images_per_request data
            if !self.images_per_request.is_empty() {
                let (min_images, max_images, avg_images, median_images) =
                    calc_stats_u64(&self.images_per_request);
                println!("\nMulti-Image Statistics:");
                println!(
                    "Min images per request: {}, Max: {}, Avg: {:.2}, Median: {}",
                    min_images, max_images, avg_images, median_images
                );

                // Count requests with multiple images
                let multi_image_requests = self
                    .images_per_request
                    .iter()
                    .filter(|&&count| count > 1)
                    .count();
                let total_requests = self.images_per_request.len();

                if total_requests > 0 {
                    let multi_image_percent =
                        (multi_image_requests as f64 / total_requests as f64) * 100.0;
                    println!(
                        "Requests with multiple images: {} ({:.1}%)",
                        multi_image_requests, multi_image_percent
                    );
                }
            }

            let total_time = self.start_time.elapsed().as_secs_f64();
            println!("Images/Second: {:.2}", total_images as f64 / total_time);
            println!("Average Size/Image: {:.2} KB", img_size_avg / 1024.0);
        }

        println!("\nScenario: {}", self.scenario);
        println!("Version: {}", self.version);
    }
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
        Err(e) => return Err(metrumbench::error::RequestError::from_reqwest(&e).into()),
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
        return Err(metrumbench::error::RequestError::from_status(status.as_u16()).into());
    }

    let image_stats: Vec<(u64, (u32, u32))> = images
        .iter()
        .map(|img| (img.size_bytes, (img.width, img.height)))
        .collect();

    if streaming {
        let mut stream = response.bytes_stream();
        let mut parser = metrumbench::sse::SseParser::new();
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
            let bytes = item.map_err(|e| metrumbench::error::RequestError::from_reqwest(&e))?;
            for event in parser.feed(&bytes) {
                match event {
                    metrumbench::sse::SseEvent::Done => done = true,
                    metrumbench::sse::SseEvent::Json(parsed) => {
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
                                if metrumbench::sse::choice_finish_reason(choice).is_some() {
                                    saw_finish = true;
                                }
                                if first_token_time.is_none()
                                    && metrumbench::sse::choice_has_output_token(choice)
                                {
                                    first_token_time = Some(start_time.elapsed());
                                }
                                if metrumbench::sse::choice_has_reasoning_token(choice)
                                    && first_reasoning_time.is_none()
                                {
                                    first_reasoning_time = Some(start_time.elapsed());
                                }
                                if metrumbench::sse::choice_has_output_token(choice) {
                                    let now = start_time.elapsed();
                                    if let Some(previous) = previous_token_time {
                                        itl.push(now.saturating_sub(previous));
                                    }
                                    previous_token_time = Some(now);
                                }
                                if let Some(content) = metrumbench::sse::choice_output_text(choice)
                                {
                                    completion_text.push_str(content);
                                }
                            }
                        }
                    }
                    metrumbench::sse::SseEvent::Raw(_) => {}
                }
            }
            if done {
                break;
            }
        }
        if !done && !saw_finish {
            return Err(metrumbench::error::RequestError::StreamTruncated.into());
        }
        let Some(ttft) = first_token_time else {
            return Err(metrumbench::error::RequestError::NoOutputToken.into());
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

fn create_log_record(args: &Args, metrics: &Metrics, resolved: &ResolvedEndpoints) -> Value {
    let mut sorted_rt = metrics.response_times.clone();
    sorted_rt.sort();
    let mut sorted_ttft = metrics.ttft_times.clone();
    sorted_ttft.sort();
    let mut sorted_tpot = metrics.tpot_times.clone();
    sorted_tpot.sort();

    let (config_url, config_endpoint, config_endpoints) = match resolved {
        ResolvedEndpoints::Single { url, name, .. } => (url.clone(), Some(name.clone()), None),
        ResolvedEndpoints::Multi {
            endpoint_names_with_weights,
            ..
        } => {
            let names: Vec<String> = endpoint_names_with_weights
                .iter()
                .map(|(n, _)| n.clone())
                .collect();
            (String::new(), None, Some(names))
        }
    };
    let per_endpoint_json: serde_json::Map<String, Value> = metrics
        .endpoint_metrics
        .iter()
        .map(|(name, ep)| {
            let ep_requests = ep.response_times.len() + ep.errors.len();
            let ep_elapsed = metrics.start_time.elapsed().as_secs_f64();
            (
                name.clone(),
                json!({
                    "requests": ep_requests,
                    "errors": ep.errors.len(),
                    "prompt_tokens_total": ep.prompt_tokens.iter().sum::<u64>(),
                    "completion_tokens_total": ep.completion_tokens.iter().sum::<u64>(),
                    "requests_per_second": if ep_elapsed > 0.0 { ep_requests as f64 / ep_elapsed } else { 0.0 }
                }),
            )
        })
        .collect();

    json!({
        "unique_id": unique_id::generate_uuid(),
        "human_readable_id": unique_id::generate_human_readable_unique_id(3),
        "timestamp": Utc::now().to_rfc3339(),
        "metrumbench_version": VERSION,
        "compile_info": compile_time_info::get_compile_info(),
        "config": {
            "scenario": args.scenario,
            "url": config_url,
            "endpoint": config_endpoint,
            "endpoints": config_endpoints,
            "model": args.model,
            "num_requests": args.num_requests,
            "concurrency": args.concurrency,
            "max_tokens": args.max_tokens,
            "temperature": args.temperature,
            "log_level": args.log_level,
            "prompts_file": args.prompts,
            "data_log": args.data_log,
            "debug_log": args.debug_log,
            "error_log": args.error_log,
            "request_timeout": args.request_timeout,
            "connect_timeout": args.connect_timeout,
            "pool_idle_timeout": args.pool_idle_timeout,
            "tcp_keepalive": args.tcp_keepalive,
            "stop_after_seconds": args.stop_after_seconds,
            "ramp_up_seconds": args.ramp_up_seconds,
            "num_images_batch": args.num_images_batch,
            "image_cache_size": args.image_cache_size,
            "max_image_dimension": args.max_image_dimension,
            "reencode_jpeg": args.reencode_jpeg,
            "image_detail": format!("{}", args.image_detail),
            "server_side_download": args.server_side_download,
        },
        "metrics": {
            "response_times": {
                "min_ms": sorted_rt.first().unwrap_or(&Duration::default()).as_millis(),
                "max_ms": sorted_rt.last().unwrap_or(&Duration::default()).as_millis(),
                "avg_ms": (sorted_rt.iter().sum::<Duration>() / sorted_rt.len().max(1) as u32).as_millis(),
                "p50_ms": Metrics::calc_percentile(&sorted_rt, 50.0).as_millis(),
                "p90_ms": Metrics::calc_percentile(&sorted_rt, 90.0).as_millis(),
                "p95_ms": Metrics::calc_percentile(&sorted_rt, 95.0).as_millis(),
                "p99_ms": Metrics::calc_percentile(&sorted_rt, 99.0).as_millis()
            },
            "ttft": {
                "min_ms": sorted_ttft.first().unwrap_or(&Duration::default()).as_millis(),
                "max_ms": sorted_ttft.last().unwrap_or(&Duration::default()).as_millis(),
                "avg_ms": (sorted_ttft.iter().sum::<Duration>() / sorted_ttft.len().max(1) as u32).as_millis(),
                "p50_ms": Metrics::calc_percentile(&sorted_ttft, 50.0).as_millis(),
                "p90_ms": Metrics::calc_percentile(&sorted_ttft, 90.0).as_millis(),
                "p95_ms": Metrics::calc_percentile(&sorted_ttft, 95.0).as_millis(),
                "p99_ms": Metrics::calc_percentile(&sorted_ttft, 99.0).as_millis()
            },
            "tpot": {
                "min_ms": sorted_tpot.first().unwrap_or(&Duration::default()).as_millis(),
                "max_ms": sorted_tpot.last().unwrap_or(&Duration::default()).as_millis(),
                "avg_ms": (sorted_tpot.iter().sum::<Duration>() / sorted_tpot.len().max(1) as u32).as_millis(),
                "p50_ms": Metrics::calc_percentile(&sorted_tpot, 50.0).as_millis(),
                "p90_ms": Metrics::calc_percentile(&sorted_tpot, 90.0).as_millis(),
                "p95_ms": Metrics::calc_percentile(&sorted_tpot, 95.0).as_millis(),
                "p99_ms": Metrics::calc_percentile(&sorted_tpot, 99.0).as_millis()
            },
            "tokens": {
                "prompt": {
                    "total": metrics.prompt_tokens.iter().sum::<u64>(),
                    "min": metrics.prompt_tokens.iter().min().unwrap_or(&0),
                    "max": metrics.prompt_tokens.iter().max().unwrap_or(&0),
                    "avg": metrics.prompt_tokens.iter().sum::<u64>() as f64 / metrics.prompt_tokens.len().max(1) as f64,
                    "per_second": metrics.prompt_tokens.iter().sum::<u64>() as f64 / metrics.start_time.elapsed().as_secs_f64()
                },
                "completion": {
                    "total": metrics.completion_tokens.iter().sum::<u64>(),
                    "min": metrics.completion_tokens.iter().min().unwrap_or(&0),
                    "max": metrics.completion_tokens.iter().max().unwrap_or(&0),
                    "avg": metrics.completion_tokens.iter().sum::<u64>() as f64 / metrics.completion_tokens.len().max(1) as f64,
                    "per_second": metrics.completion_tokens.iter().sum::<u64>() as f64 / metrics.start_time.elapsed().as_secs_f64()
                },
                "total": {
                    "total": metrics.total_tokens.iter().sum::<u64>(),
                    "min": metrics.total_tokens.iter().min().unwrap_or(&0),
                    "max": metrics.total_tokens.iter().max().unwrap_or(&0),
                    "avg": metrics.total_tokens.iter().sum::<u64>() as f64 / metrics.total_tokens.len().max(1) as f64,
                    "per_second": metrics.total_tokens.iter().sum::<u64>() as f64 / metrics.start_time.elapsed().as_secs_f64()
                }
            },
            "errors": {
                "count": metrics.errors.len(),
                "rate": (metrics.errors.len() as f64 / args.num_requests as f64) * 100.0
            },
            "timing": {
                "total_time_seconds": metrics.start_time.elapsed().as_secs_f64(),
                "requests_per_second": args.num_requests as f64 / metrics.start_time.elapsed().as_secs_f64(),
                "successful_requests": metrics.response_times.len(),
                "failed_requests": metrics.errors.len(),
                "ramp_up_seconds": args.ramp_up_seconds,
                "steady_state_seconds": metrics.start_time.elapsed().as_secs_f64() - args.ramp_up_seconds.unwrap_or(0) as f64,
                "steady_state_requests_per_second": metrics.response_times.len() as f64 /
                    (metrics.start_time.elapsed().as_secs_f64() - args.ramp_up_seconds.unwrap_or(0) as f64).max(1.0)
            },
            "images": {
                "total_images": metrics.image_dimensions.len(),
                "multi_image_requests": metrics.images_per_request.iter().filter(|&&count| count > 1).count(),
                "total_requests_with_images": metrics.images_per_request.len(),
                "images_per_request": {
                    "min": metrics.images_per_request.iter().min().unwrap_or(&0),
                    "max": metrics.images_per_request.iter().max().unwrap_or(&0),
                    "avg": metrics.images_per_request.iter().sum::<u64>() as f64 / metrics.images_per_request.len().max(1) as f64
                }
            },
            "per_endpoint": per_endpoint_json
        }
    })
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

struct ImageCache {
    cache: LruCache<String, ImageData>,
}

impl ImageCache {
    fn new(capacity: usize) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let cap = std::num::NonZeroUsize::new(capacity).ok_or("image_cache_size must be >= 1")?;
        Ok(Self {
            cache: LruCache::new(cap),
        })
    }

    async fn get_or_load(
        &mut self,
        client: &Client,
        path: &str,
        max_dimension: Option<u32>,
        timeout_secs: u64,
        reencode_jpeg: bool,
    ) -> Result<ImageData, Box<dyn Error + Send + Sync>> {
        if let Some(data) = self.cache.get(path) {
            debug!("Cache hit for image: {}", path);
            return Ok(data.clone());
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

        self.cache.put(path.to_string(), image_data.clone());
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
    metrumbench::banner::print_banner_metrumbench(VERSION, "metrum-ai-bench-vlm");

    let args = Args::parse();

    // Check for version-only flag first
    if args.version_only {
        println!("metrum-ai-bench-vlm version {}", VERSION);
        return Ok(());
    }

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
        let offset = metrumbench::timecheck::check_ntp_offset();
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

    let client =
        metrumbench::http_client::build_http_client(metrumbench::http_client::HttpClientOptions {
            request_timeout: Some(Duration::from_secs(args.request_timeout)),
            connect_timeout: Duration::from_secs(args.connect_timeout),
            pool_max_idle_per_host: args.concurrency as usize,
            pool_idle_timeout: Duration::from_secs(args.pool_idle_timeout),
            tcp_keepalive: Duration::from_secs(args.tcp_keepalive),
            ca_cert: args.common.ca_cert.as_deref().map(std::path::Path::new),
            insecure: args.common.insecure,
        })?;

    let mut records = load_metrumbench_vlm_records(&args.prompts)?;
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
    let mut metrics = Metrics::new(args.scenario.clone());
    let semaphore = Arc::new(Semaphore::new(
        args.common.max_concurrency.unwrap_or(args.concurrency) as usize,
    ));
    let endpoint_selector = Arc::new(metrumbench::endpoints::EndpointSelector::new(
        &resolved_endpoints,
    ));
    let sink = Arc::new(metrumbench::jsonl::JsonlSink::create(&args.data_log)?);
    let stop = metrumbench::runner::StopFlag::new();
    metrumbench::runner::install_stop_handlers(stop.clone());
    let arrival_kind = args.common.arrival_kind();
    let mut handles = vec![];
    let mut completed = 0;
    let mut last_percentage = 0;
    let mut metrics_started = false;

    let start_time = Instant::now();
    let ramp_up_start = start_time;
    let mut current_concurrency;
    let (record_tx, mut record_rx) =
        tokio::sync::mpsc::unbounded_channel::<metrumbench::record::RequestRecord>();

    let mut arrival_rng = rand::rngs::StdRng::seed_from_u64(args.common.seed.wrapping_add(1));
    let slots = metrumbench::load::schedule(
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
        let queue_delay = metrumbench::runner::queue_delay_for_slot(
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
                metrics.record_error(
                    &endpoint_name,
                    "server_side_download requires http(s) URLs; local paths and file:// are not sent".to_string(),
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
                            metrics.record_error(&endpoint_name, e.to_string());
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
                            metrics.record_error(&endpoint_name, e.to_string());
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
                                metrics.record_error(&endpoint_name, e.to_string());
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
                        metrics.record_error(&endpoint_name, e.to_string());
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
            &metrumbench::args_common::CommonBenchArgs::unique_prompt(
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
                metrics.record_error(&endpoint_name, e.to_string());
                drop(permit);
                continue 'request_loop;
            }
        };

        let request_timeout = args.request_timeout;
        let streaming = args.streaming;
        let selected_images_clone = selected_images.clone();
        let phase = metrumbench::record::Phase::for_seq(slot.seq, args.common.warmup_requests);
        let seq = slot.seq;
        let scheduled_delay = slot.scheduled_delay;
        let record_schedule = metrumbench::runner::should_record_schedule(arrival_kind);
        let sink_task = sink.clone();
        let record_tx = record_tx.clone();
        let run_id_task = run_id.clone();
        let tokenizer_path = args.common.tokenizer.clone();
        let prompt_text = metrumbench::args_common::CommonBenchArgs::unique_prompt(
            &selected_record.0,
            i as u64,
            args.common.unique_prompts,
            args.common.seed,
            &run_id,
        );
        let handle = tokio::spawn(async move {
            let started_at = Utc::now();
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

            let tokenizer = match metrumbench::tokenizer::LocalTokenizer::from_file(
                tokenizer_path.as_deref(),
            ) {
                Ok(t) => t,
                Err(_) => metrumbench::tokenizer::LocalTokenizer::from_file(None)
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
                        metrumbench::runner::completed_at_from_start(started_at, response_time);
                    let tokenized_prompt_tokens = tokenizer.count(&prompt_text).ok().flatten();
                    let tokenized_completion_tokens =
                        tokenizer.count(&completion_text).ok().flatten();
                    let usage_missing = completion_tokens == 0 && !completion_text.is_empty();
                    let mut rec = metrumbench::record::RequestRecord::success(
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
                    .with_first_byte(first_byte);
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
                    let latency = Utc::now()
                        .signed_duration_since(started_at)
                        .to_std()
                        .unwrap_or(Duration::ZERO);
                    let completed_at =
                        metrumbench::runner::completed_at_from_start(started_at, latency);
                    let request_error = metrumbench::error::RequestError::from_error(e.as_ref());
                    if matches!(request_error, metrumbench::error::RequestError::Connect) {
                        endpoint_selector.note_connect_failure(&endpoint_name);
                    }
                    let mut rec = metrumbench::record::RequestRecord::failed(
                        seq,
                        phase,
                        endpoint_name.clone(),
                        started_at,
                        completed_at,
                        latency,
                        request_error,
                    );
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
    let mut records: Vec<metrumbench::record::RequestRecord> = Vec::new();
    while let Some(rec) = record_rx.recv().await {
        let endpoint_name = rec.endpoint.clone();
        let phase = rec.phase;
        if rec.is_success() {
            let response_time = Duration::from_secs_f64(rec.latency_s);
            let ttft = rec.ttft_s.map(Duration::from_secs_f64);
            let prompt_tokens = rec.prompt_tokens;
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
            if phase == metrumbench::record::Phase::Warmup {
                records.push(rec);
                continue;
            }

            let tpot = match ttft {
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
            metrics.record_success(
                &endpoint_name,
                response_time,
                ttft,
                tpot,
                prompt_tokens,
                completion_tokens,
                total_tokens,
                &image_stats,
            );
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
            if (args.ramp_up_seconds.is_none() || metrics_started)
                && phase != metrumbench::record::Phase::Warmup
            {
                metrics.record_error(&endpoint_name, err_msg);
            }
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
    metrics.print_stats(&resolved_endpoints);

    let window_seconds = metrumbench::runner::window_seconds_from_records(&records);
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
    let mut shared_summary = metrumbench::summary::RunSummary::from_records_with_options(
        &records,
        window_seconds,
        stop.is_stopped(),
        &slos,
        args.common.throughput_bin_seconds,
    )
    .with_config(metrumbench::summary::EffectiveRunConfig {
        run_id: run_id.clone(),
        common: (&args.common).into(),
        effective_system_prompt: vlm_system,
        body_template,
        unique_prompt_nonce_template:
            metrumbench::args_common::CommonBenchArgs::unique_prompt_nonce_template(
                args.common.unique_prompts,
            ),
    });
    shared_summary.environment =
        metrumbench::environment::collect(ntp_offset_ms, Some(args.model.clone()));
    if let Err(e) = sink.write(&shared_summary) {
        warn!("Failed to write summary JSONL: {e}");
    }
    let summary = create_log_record(&args, &metrics, &resolved_endpoints);
    if let Err(e) = sink.write(&summary) {
        return Err(format!("Failed to write to data log: {e}").into());
    }

    if args.common.fail_on_error && !metrics.errors.is_empty() {
        Err("Test completed with errors".into())
    } else {
        Ok(())
    }
}
