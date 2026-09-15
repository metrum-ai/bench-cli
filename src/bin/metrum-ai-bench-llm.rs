// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0
#![allow(clippy::too_many_arguments)]

use chrono::Utc;
use clap::Parser;
use futures_util::StreamExt;
use log::{debug, error, info, trace, warn};
use metrumbench::compile_time_info;
use metrumbench::endpoints::{resolve_endpoints, ResolvedEndpoints};
use metrumbench::prompt_inputs::load_metrumbench_llm_prompts;
use metrumbench::unique_id;
use rand::seq::SliceRandom;
use rand::SeedableRng;
use reqwest::{Client, Response};
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

fn effective_ramp_up_seconds(ramp_up_seconds: Option<u64>) -> Option<u64> {
    ramp_up_seconds.filter(|seconds| *seconds > 0)
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum MetrumBenchLLMMode {
    Chat,
    Completion,
}

impl std::fmt::Display for MetrumBenchLLMMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MetrumBenchLLMMode::Chat => write!(f, "chat"),
            MetrumBenchLLMMode::Completion => write!(f, "completion"),
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
    mode: MetrumBenchLLMMode,

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
    common: metrumbench::args_common::CommonBenchArgs,

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

/// Per-endpoint metrics (same shape as aggregate but keyed by endpoint name).
#[derive(Default, Clone)]
struct EndpointMetrics {
    response_times: Vec<Duration>,
    ttft_times: Vec<Duration>,
    tpot_times: Vec<f64>,
    prompt_tokens: Vec<u64>,
    completion_tokens: Vec<u64>,
    total_tokens: Vec<u64>,
    prompt_words: Vec<usize>,
    completion_words: Vec<usize>,
    errors: Vec<String>,
    error_types: HashMap<String, usize>,
}

/// Structure to collect and analyze performance metrics during load testing.
///
/// This struct maintains various timing, token, and error metrics to provide
/// comprehensive performance analysis of the AI model endpoint.
struct Metrics {
    /// Response times for all successful requests
    response_times: Vec<Duration>,
    /// Time to first token for all successful requests
    ttft_times: Vec<Duration>,
    /// Time per output token for all successful requests (in seconds)
    tpot_times: Vec<f64>,
    /// Number of prompt tokens for each request
    prompt_tokens: Vec<u64>,
    /// Number of completion tokens for each request
    completion_tokens: Vec<u64>,
    /// Total tokens (prompt + completion) for each request
    total_tokens: Vec<u64>,
    /// Number of words in each prompt
    prompt_words: Vec<usize>,
    /// Number of words in each completion
    completion_words: Vec<usize>,
    /// Start time of the load test
    start_time: Instant,
    /// Start time of metrics collection (after ramp-up)
    metrics_start_time: Option<Instant>,
    /// Detailed error messages for failed requests
    errors: Vec<String>,
    /// Error types and their counts
    error_types: HashMap<String, usize>,
    /// Timestamps of errors for temporal analysis
    error_timestamps: Vec<(String, Instant)>,
    /// Description of the load test scenario
    scenario: String,
    /// Version of metrumbench
    version: String,
    /// Per-endpoint metrics (keyed by endpoint name)
    endpoint_metrics: HashMap<String, EndpointMetrics>,
}

impl Metrics {
    /// Creates a new Metrics instance for the given scenario.
    fn new(scenario: String) -> Self {
        Self {
            response_times: Vec::new(),
            ttft_times: Vec::new(),
            tpot_times: Vec::new(),
            prompt_tokens: Vec::new(),
            completion_tokens: Vec::new(),
            total_tokens: Vec::new(),
            prompt_words: Vec::new(),
            completion_words: Vec::new(),
            start_time: Instant::now(),
            metrics_start_time: None,
            errors: Vec::new(),
            error_types: HashMap::new(),
            error_timestamps: Vec::new(),
            scenario,
            version: VERSION.to_string(),
            endpoint_metrics: HashMap::new(),
        }
    }

    /// Records a successful request for the given endpoint (and aggregate).
    fn record_success(
        &mut self,
        endpoint_name: &str,
        response_time: Duration,
        ttft: Duration,
        tpot: Option<f64>,
        prompt_tokens: u64,
        completion_tokens: u64,
        total_tokens: u64,
        prompt_words: usize,
        completion_words: usize,
    ) {
        self.response_times.push(response_time);
        self.ttft_times.push(ttft);
        if let Some(t) = tpot {
            self.tpot_times.push(t);
        }
        self.prompt_tokens.push(prompt_tokens);
        self.completion_tokens.push(completion_tokens);
        self.total_tokens.push(total_tokens);
        self.prompt_words.push(prompt_words);
        self.completion_words.push(completion_words);
        let ep = self
            .endpoint_metrics
            .entry(endpoint_name.to_string())
            .or_default();
        ep.response_times.push(response_time);
        ep.ttft_times.push(ttft);
        if let Some(t) = tpot {
            ep.tpot_times.push(t);
        }
        ep.prompt_tokens.push(prompt_tokens);
        ep.completion_tokens.push(completion_tokens);
        ep.total_tokens.push(total_tokens);
        ep.prompt_words.push(prompt_words);
        ep.completion_words.push(completion_words);
    }

    /// Records an error with its type and timestamp (aggregate and per-endpoint).
    fn record_error(&mut self, endpoint_name: &str, error: String) {
        // Extract error type from the error message
        let error_type = error.split(':').next().unwrap_or("unknown").to_string();

        // Record error type
        *self.error_types.entry(error_type.clone()).or_insert(0) += 1;

        // Record error with timestamp for temporal analysis
        self.error_timestamps
            .push((error_type.clone(), Instant::now()));

        // Record full error message
        self.errors.push(error.clone());
        // Per-endpoint
        let ep = self
            .endpoint_metrics
            .entry(endpoint_name.to_string())
            .or_default();
        *ep.error_types.entry(error_type).or_insert(0) += 1;
        ep.errors.push(error);
    }

    /// Calculates the percentile value from a sorted list of durations.
    fn calc_percentile(sorted_values: &[Duration], percentile: f64) -> Duration {
        if sorted_values.is_empty() {
            return Duration::default();
        }
        let index = (((sorted_values.len() - 1) as f64 * percentile / 100.0).round() as usize)
            .min(sorted_values.len() - 1);
        *sorted_values.get(index).unwrap_or(&Duration::default())
    }

    /// Calculates basic statistics for a list of usize values.
    fn calc_stats_usize(&self, values: &[usize]) -> (usize, usize, f64, usize) {
        if values.is_empty() {
            return (0, 0, 0.0, 0);
        }
        let mut sorted = values.to_vec();
        sorted.sort();
        let len = sorted.len();
        let avg = sorted.iter().sum::<usize>() as f64 / len as f64;
        (
            *sorted.first().unwrap_or(&0),
            *sorted.last().unwrap_or(&0),
            avg,
            sorted[len / 2],
        )
    }

    /// Prints one compact block (per-endpoint or aggregate style).
    fn print_compact_block(
        &self,
        label: &str,
        response_times: &[Duration],
        ttft_times: &[Duration],
        tpot_times: &[f64],
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
        let pt_rate = total_pt as f64 / elapsed_secs;
        let ct_rate = total_ct as f64 / elapsed_secs;
        println!("\n=== {} ===", label);
        println!("  Requests:    {}", requests);
        println!("  Errors:      {}", errors.len());
        if !response_times.is_empty() {
            let mut sorted_rt = response_times.to_vec();
            sorted_rt.sort();
            let avg_rt: Duration = Duration::from_secs_f64(
                sorted_rt.iter().map(|d| d.as_secs_f64()).sum::<f64>() / sorted_rt.len() as f64,
            );
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
            let avg_ttft: Duration = Duration::from_secs_f64(
                ttft_times.iter().map(|d| d.as_secs_f64()).sum::<f64>() / ttft_times.len() as f64,
            );
            println!("  Avg TTFT:    {:.3}s", avg_ttft.as_secs_f64());
        }
        if !tpot_times.is_empty() {
            let avg_tpot = tpot_times.iter().sum::<f64>() / tpot_times.len() as f64;
            println!("  Avg TPOT:    {:.3}s", avg_tpot);
        }
        println!(
            "  Tokens:      {} prompt, {} completion",
            total_pt, total_ct
        );
        println!(
            "  Token Rate: {:.1} tokens/sec (prompt: {:.1}, completion: {:.1})",
            token_rate, pt_rate, ct_rate
        );
        println!("  Req Rate:   {:.1} req/sec", req_rate);
    }

    /// Prints comprehensive statistics about the load test.
    fn print_stats(&self, resolved: &ResolvedEndpoints) {
        let metrics_elapsed_secs = if let Some(metrics_start) = self.metrics_start_time {
            metrics_start.elapsed().as_secs_f64()
        } else {
            self.start_time.elapsed().as_secs_f64()
        };

        let names_with_weights = resolved.endpoint_names_for_display();
        let multi = names_with_weights.len() > 1;

        if multi {
            for (name, weight) in &names_with_weights {
                if let Some(ep) = self.endpoint_metrics.get(name) {
                    self.print_compact_block(
                        &format!("Endpoint: {} (weight: {})", name, weight),
                        &ep.response_times,
                        &ep.ttft_times,
                        &ep.tpot_times,
                        &ep.prompt_tokens,
                        &ep.completion_tokens,
                        &ep.total_tokens,
                        &ep.errors,
                        metrics_elapsed_secs,
                    );
                }
            }
            self.print_compact_block(
                "AGGREGATE",
                &self.response_times,
                &self.ttft_times,
                &self.tpot_times,
                &self.prompt_tokens,
                &self.completion_tokens,
                &self.total_tokens,
                &self.errors,
                metrics_elapsed_secs,
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
            let avg = {
                let s = sorted.iter().map(|d| d.as_secs_f64()).sum::<f64>();
                Duration::from_secs_f64(s / (len as f64))
            };
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

        let calc_stats_f64 = |values: &[f64]| {
            if values.is_empty() {
                return (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
            }
            let mut sorted = values.to_vec();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let len = sorted.len();
            let avg = sorted.iter().sum::<f64>() / len as f64;
            let p50_idx = (((len - 1) as f64 * 50.0 / 100.0).round() as usize).min(len - 1);
            let p90_idx = (((len - 1) as f64 * 90.0 / 100.0).round() as usize).min(len - 1);
            let p95_idx = (((len - 1) as f64 * 95.0 / 100.0).round() as usize).min(len - 1);
            let p99_idx = (((len - 1) as f64 * 99.0 / 100.0).round() as usize).min(len - 1);
            (
                *sorted.first().unwrap_or(&0.0),
                *sorted.last().unwrap_or(&0.0),
                avg,
                sorted[p50_idx],
                sorted[p90_idx],
                sorted[p95_idx],
                sorted[p99_idx],
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
                calc_stats_f64(&self.tpot_times);
            println!("\nTime Per Output Token Statistics:");
            println!(
                "Min: {:.6}s, Max: {:.6}s, Avg: {:.6}s",
                tpot_min, tpot_max, tpot_avg
            );
            println!(
                "p50: {:.6}s, p90: {:.6}s, p95: {:.6}s, p99: {:.6}s",
                tpot_p50, tpot_p90, tpot_p95, tpot_p99
            );
        }

        // Add word count statistics
        println!("\nWord Count Statistics:");
        if !self.prompt_words.is_empty() {
            let (pw_min, pw_max, pw_avg, pw_median) = self.calc_stats_usize(&self.prompt_words);
            let total_prompt_words: usize = self.prompt_words.iter().sum();
            let word_elapsed = if let Some(metrics_start) = self.metrics_start_time {
                metrics_start.elapsed().as_secs_f64()
            } else {
                self.start_time.elapsed().as_secs_f64()
            };
            let prompt_words_per_second = total_prompt_words as f64 / word_elapsed;
            println!("Prompt Words:");
            println!(
                "  Min: {}, Max: {}, Avg: {:.2}, Median: {}",
                pw_min, pw_max, pw_avg, pw_median
            );
            println!("  Total Prompt Words: {}", total_prompt_words);
            println!("  Words Per Second: {:.2}", prompt_words_per_second);
        }

        if !self.completion_words.is_empty() {
            let (cw_min, cw_max, cw_avg, cw_median) = self.calc_stats_usize(&self.completion_words);
            let total_completion_words: usize = self.completion_words.iter().sum();
            let word_elapsed = if let Some(metrics_start) = self.metrics_start_time {
                metrics_start.elapsed().as_secs_f64()
            } else {
                self.start_time.elapsed().as_secs_f64()
            };
            let completion_words_per_second = total_completion_words as f64 / word_elapsed;
            println!("Completion Words:");
            println!(
                "  Min: {}, Max: {}, Avg: {:.2}, Median: {}",
                cw_min, cw_max, cw_avg, cw_median
            );
            println!("  Total Completion Words: {}", total_completion_words);
            println!("  Words Per Second: {:.2}", completion_words_per_second);
        }

        // Add error analysis
        println!("\nError Analysis:");
        println!("Total Errors: {}", self.errors.len());

        // Error type distribution
        println!("\nError Type Distribution:");
        if !self.errors.is_empty() {
            for (error_type, count) in &self.error_types {
                println!(
                    "  {}: {} ({:.1}%)",
                    error_type,
                    count,
                    (count * 100) as f64 / self.errors.len() as f64
                );
            }
        } else {
            println!("  No errors to analyze");
        }

        // Temporal error analysis
        if let (Some(first), Some(last)) =
            (self.error_timestamps.first(), self.error_timestamps.last())
        {
            let first_error = first.1;
            let last_error = last.1;
            let error_duration = last_error.duration_since(first_error);

            println!("\nError Timing Analysis:");
            println!(
                "  First Error: {:?} after start",
                first_error.duration_since(self.start_time)
            );
            println!(
                "  Last Error: {:?} after start",
                last_error.duration_since(self.start_time)
            );
            println!("  Error Duration: {:?}", error_duration);
            let errors_per_second = if error_duration.as_secs_f64() > 0.0 {
                self.errors.len() as f64 / error_duration.as_secs_f64()
            } else {
                0.0
            };
            println!("  Average Errors/Second: {:.2}", errors_per_second);
        }

        // Rest of the timing statistics...
        let total_time = self.start_time.elapsed();
        let successful_requests = self.response_times.len();
        let total_requests = successful_requests + self.errors.len();

        // Use metrics collection time if available (excludes ramp-up)
        let metrics_elapsed = if let Some(metrics_start) = self.metrics_start_time {
            metrics_start.elapsed()
        } else {
            self.start_time.elapsed()
        };

        // Calculate rates only if we have a non-zero elapsed time
        let elapsed_secs = metrics_elapsed.as_secs_f64();
        if elapsed_secs > 0.0 {
            let requests_per_second = total_requests as f64 / elapsed_secs;
            let tokens_per_second = self.total_tokens.iter().sum::<u64>() as f64 / elapsed_secs;
            let prompt_tokens_per_second =
                self.prompt_tokens.iter().sum::<u64>() as f64 / elapsed_secs;
            let completion_tokens_per_second =
                self.completion_tokens.iter().sum::<u64>() as f64 / elapsed_secs;

            println!("\nTiming Statistics:");
            println!("Total Time: {:?}", total_time);
            if self.metrics_start_time.is_some() {
                println!("Metrics Collection Time: {:?}", metrics_elapsed);
            }
            if successful_requests > 0 {
                let avg_latency = {
                    let s = self
                        .response_times
                        .iter()
                        .map(|d| d.as_secs_f64())
                        .sum::<f64>();
                    Duration::from_secs_f64(s / (successful_requests as f64))
                };
                println!("Average Request Latency: {:?}", avg_latency);
            }
            println!(
                "Requests/Second: {:.2} (successful: {}, failed: {})",
                requests_per_second,
                successful_requests,
                self.errors.len()
            );
            println!("Prompt Tokens/Second: {:.2}", prompt_tokens_per_second);
            println!(
                "Completion Tokens/Second: {:.2}",
                completion_tokens_per_second
            );
            println!("Total Tokens/Second: {:.2}", tokens_per_second);
        } else {
            println!("\nTiming Statistics:");
            println!("Total Time: {:?}", total_time);
            println!("Not enough data to calculate rates (test duration too short)");
        }

        println!("\nScenario: {}", self.scenario);
        println!("Version: {}", self.version);
    }
}

/// Classifies streaming errors with proper context and error chaining
#[allow(dead_code)]
fn classify_stream_error(e: &reqwest::Error, chunks_processed: usize) -> String {
    let mut context = Vec::new();

    // Classify by reqwest error kind
    if e.is_timeout() {
        context.push("Network timeout".to_string());
    }
    if e.is_connect() {
        context.push("Connection failure".to_string());
    }
    if e.is_body() {
        context.push("Body processing error".to_string());
    }
    if e.is_decode() {
        context.push("Response decoding error".to_string());
    }
    if e.is_redirect() {
        context.push("Redirect error".to_string());
    }
    if e.is_request() {
        context.push("Request construction error".to_string());
    }

    // Add specific error details
    if let Some(url) = e.url() {
        context.push(format!("Failed URL: {}", url));
    }
    if let Some(status) = e.status() {
        context.push(format!("HTTP status: {}", status));
    }

    // Add stream context
    context.push(format!("Chunks processed: {}", chunks_processed));

    // If no specific classification, use the error message
    if context.is_empty() {
        context.push(format!("Unknown error: {}", e));
    }

    context.join(" | ")
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
    ttft: Duration,
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
    mode: MetrumBenchLLMMode,
    streaming: bool,
    request_timeout: u64,
    api_key: &str,
) -> Result<StreamMetrics, Box<dyn Error + Send + Sync>> {
    let start_time = Instant::now();

    // Get prompt text for word counting (chat: messages[].content, completion: prompt)
    let prompt = match mode {
        MetrumBenchLLMMode::Chat => payload["messages"]
            .as_array()
            .and_then(|msgs| msgs.last())
            .and_then(|msg| msg["content"].as_str())
            .unwrap_or(""),
        MetrumBenchLLMMode::Completion => payload["prompt"].as_str().unwrap_or(""),
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
                // Log comprehensive error details
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
                return Err(anyhow::anyhow!("HTTP request failed: {}", e)
                    .context("Streaming request failed")
                    .into());
            }
        };

        // Check for HTTP errors
        if !response.status().is_success() {
            let status = response.status();
            let headers = response.headers().clone();
            let error_body = response
                .text()
                .await
                .unwrap_or_else(|_| "No error body".to_string());

            // Classify error type for metrics
            let error_type = match status.as_u16() {
                429 => "rate_limit",
                401 | 403 => "authentication",
                400 => "bad_request",
                500..=599 => "server_error",
                _ => "unknown_error",
            };

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
            return Err(anyhow::anyhow!("HTTP error: {} - {}", status, error_body)
                .context(format!("Request failed with status {}", status))
                .context(format!("Error type: {}", error_type))
                .into());
        }

        trace!("Request started streaming");
        let mut stream = response.bytes_stream();
        let mut parser = metrumbench::sse::SseParser::new();
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
            let bytes = item.map_err(|e| anyhow::anyhow!("stream error: {e}"))?;
            for event in parser.feed(&bytes) {
                match event {
                    metrumbench::sse::SseEvent::Done => {
                        done = true;
                    }
                    metrumbench::sse::SseEvent::Json(parsed) => {
                        if let Some(error) = parsed.get("error") {
                            return Err(anyhow::anyhow!("API error in stream: {}", error).into());
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
                                if metrumbench::sse::choice_finish_reason(choice).is_some() {
                                    saw_finish = true;
                                }
                                if first_reasoning_time.is_none()
                                    && metrumbench::sse::choice_has_reasoning_token(choice)
                                {
                                    first_reasoning_time = Some(start_time.elapsed());
                                }
                                if metrumbench::sse::choice_has_output_token(choice) {
                                    let now = Instant::now();
                                    if first_token_time.is_none() {
                                        first_token_time = Some(start_time.elapsed());
                                    } else if let Some(prev) = last_token_at {
                                        itl.push(now.saturating_duration_since(prev));
                                    }
                                    last_token_at = Some(now);
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
            return Err(anyhow::anyhow!("stream truncated").into());
        }
        let Some(ttft) = first_token_time else {
            return Err(anyhow::anyhow!("no output token").into());
        };
        trace!("Request completed successfully");
        let completion_word_count = count_words(&completion_text);
        Ok(StreamMetrics {
            latency: start_time.elapsed(),
            ttft,
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
        let response: Response = client
            .post(url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&payload)
            .timeout(Duration::from_secs(request_timeout))
            .send()
            .await?;

        let total_time = start_time.elapsed();
        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_else(|_| String::new());
            let error_type = match status.as_u16() {
                429 => "rate_limit",
                401 | 403 => "authentication",
                400 => "bad_request",
                500..=599 => "server_error",
                _ => "unknown_error",
            };
            return Err(anyhow::anyhow!("HTTP {}: {}", status, error_body.trim())
                .context(format!("Error type: {}", error_type))
                .into());
        }
        let json_resp: Value = response.json().await?;

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
            MetrumBenchLLMMode::Chat => json_resp
                .get("choices")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("message"))
                .and_then(|m| m.get("content"))
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            MetrumBenchLLMMode::Completion => json_resp
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

        Ok(StreamMetrics {
            latency: total_time,
            ttft: total_time,
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

fn create_log_record(args: &Args, metrics: &Metrics, resolved: &ResolvedEndpoints) -> Value {
    let mut sorted_rt = metrics.response_times.clone();
    sorted_rt.sort();
    let mut sorted_ttft = metrics.ttft_times.clone();
    sorted_ttft.sort();
    let mut sorted_tpot = metrics.tpot_times.clone();
    sorted_tpot.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    // Calculate averages for Duration values outside the json! macro
    let rt_avg_ms = if !sorted_rt.is_empty() {
        let s = sorted_rt.iter().map(|d| d.as_secs_f64()).sum::<f64>();
        Duration::from_secs_f64(s / (sorted_rt.len() as f64)).as_millis()
    } else {
        0
    };

    let ttft_avg_ms = if !sorted_ttft.is_empty() {
        let s = sorted_ttft.iter().map(|d| d.as_secs_f64()).sum::<f64>();
        Duration::from_secs_f64(s / (sorted_ttft.len() as f64)).as_millis()
    } else {
        0
    };

    // Helper to calculate f64 percentiles
    let calc_percentile_f64 = |sorted: &[f64], percentile: f64| -> f64 {
        if sorted.is_empty() {
            return 0.0;
        }
        let idx = (((sorted.len() - 1) as f64 * percentile / 100.0).round() as usize)
            .min(sorted.len() - 1);
        sorted[idx]
    };

    let calc_percentile_u64 = |values: &[u64], percentile: f64| -> u64 {
        if values.is_empty() {
            return 0;
        }
        let mut sorted = values.to_vec();
        sorted.sort();
        let idx = (((sorted.len() - 1) as f64 * percentile / 100.0).round() as usize)
            .min(sorted.len() - 1);
        sorted[idx]
    };

    // Use the Metrics method directly
    let (prompt_min, prompt_max, prompt_avg, _) = metrics.calc_stats_usize(&metrics.prompt_words);
    let (completion_min, completion_max, completion_avg, _) =
        metrics.calc_stats_usize(&metrics.completion_words);

    // Determine how many requests actually completed (success or error)
    let total_completed_requests = metrics.response_times.len() + metrics.errors.len();
    let oom_occurred = metrics
        .errors
        .iter()
        .any(|error| error_message_indicates_oom(error));

    // Calculate metrics elapsed time (excluding ramp-up if applicable)
    let metrics_elapsed_secs = if let Some(metrics_start) = metrics.metrics_start_time {
        metrics_start.elapsed().as_secs_f64()
    } else {
        metrics.start_time.elapsed().as_secs_f64()
    };

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
            let ep_elapsed = metrics_elapsed_secs;
            let ep_requests = ep.response_times.len() + ep.errors.len();
            let ep_sorted_rt: Vec<Duration> = {
                let mut v = ep.response_times.clone();
                v.sort();
                v
            };
            let ep_sorted_ttft: Vec<Duration> = {
                let mut v = ep.ttft_times.clone();
                v.sort();
                v
            };
            let ep_sorted_tpot: Vec<f64> = {
                let mut v = ep.tpot_times.clone();
                v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                v
            };
            (
                name.clone(),
                json!({
                    "response_times": {
                        "min_ms": ep_sorted_rt.first().map(|d| d.as_millis()).unwrap_or(0),
                        "max_ms": ep_sorted_rt.last().map(|d| d.as_millis()).unwrap_or(0),
                        "avg_ms": if ep_sorted_rt.is_empty() { 0.0 } else {
                            ep_sorted_rt.iter().map(|d| d.as_secs_f64()).sum::<f64>() / ep_sorted_rt.len() as f64 * 1000.0
                        },
                        "p50_ms": Metrics::calc_percentile(&ep_sorted_rt, 50.0).as_millis(),
                        "p99_ms": Metrics::calc_percentile(&ep_sorted_rt, 99.0).as_millis()
                    },
                    "ttft": {
                        "avg_ms": if ep_sorted_ttft.is_empty() { 0.0 } else {
                            ep_sorted_ttft.iter().map(|d| d.as_secs_f64()).sum::<f64>() / ep_sorted_ttft.len() as f64 * 1000.0
                        }
                    },
                    "tpot": {
                        "avg_s": if ep_sorted_tpot.is_empty() { 0.0 } else {
                            ep_sorted_tpot.iter().sum::<f64>() / ep_sorted_tpot.len() as f64
                        }
                    },
                    "tokens": {
                        "prompt_total": ep.prompt_tokens.iter().sum::<u64>(),
                        "prompt_p50": calc_percentile_u64(&ep.prompt_tokens, 50.0),
                        "prompt_p95": calc_percentile_u64(&ep.prompt_tokens, 95.0),
                        "completion_total": ep.completion_tokens.iter().sum::<u64>(),
                        "completion_p50": calc_percentile_u64(&ep.completion_tokens, 50.0),
                        "completion_p95": calc_percentile_u64(&ep.completion_tokens, 95.0),
                        "total": ep.total_tokens.iter().sum::<u64>(),
                        "total_p50": calc_percentile_u64(&ep.total_tokens, 50.0),
                        "total_p95": calc_percentile_u64(&ep.total_tokens, 95.0),
                        "prompt_per_second": if ep_elapsed > 0.0 { ep.prompt_tokens.iter().sum::<u64>() as f64 / ep_elapsed } else { 0.0 },
                        "completion_per_second": if ep_elapsed > 0.0 { ep.completion_tokens.iter().sum::<u64>() as f64 / ep_elapsed } else { 0.0 }
                    },
                    "errors": {
                        "count": ep.errors.len(),
                        "types": &ep.error_types,
                        "oom_occurred": ep
                            .errors
                            .iter()
                            .any(|error| error_message_indicates_oom(error))
                    },
                    "requests": ep_requests,
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
            "mode": format!("{}", args.mode),
            "streaming": args.streaming,
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
        },
        "metrics": {
            "words": {
                "prompt": {
                    "total": metrics.prompt_words.iter().sum::<usize>(),
                    "min": prompt_min,
                    "max": prompt_max,
                    "avg": prompt_avg,
                    "per_second": if metrics_elapsed_secs > 0.0 {
                        metrics.prompt_words.iter().sum::<usize>() as f64 / metrics_elapsed_secs
                    } else { 0.0 }
                },
                "completion": {
                    "total": metrics.completion_words.iter().sum::<usize>(),
                    "min": completion_min,
                    "max": completion_max,
                    "avg": completion_avg,
                    "per_second": if metrics_elapsed_secs > 0.0 {
                        metrics.completion_words.iter().sum::<usize>() as f64 / metrics_elapsed_secs
                    } else { 0.0 }
                }
            },
            "response_times": {
                "min_ms": sorted_rt.first().unwrap_or(&Duration::default()).as_millis(),
                "max_ms": sorted_rt.last().unwrap_or(&Duration::default()).as_millis(),
                "avg_ms": rt_avg_ms,
                "p50_ms": Metrics::calc_percentile(&sorted_rt, 50.0).as_millis(),
                "p90_ms": Metrics::calc_percentile(&sorted_rt, 90.0).as_millis(),
                "p95_ms": Metrics::calc_percentile(&sorted_rt, 95.0).as_millis(),
                "p99_ms": Metrics::calc_percentile(&sorted_rt, 99.0).as_millis()
            },
            "ttft": {
                "min_ms": sorted_ttft.first().unwrap_or(&Duration::default()).as_millis(),
                "max_ms": sorted_ttft.last().unwrap_or(&Duration::default()).as_millis(),
                "avg_ms": ttft_avg_ms,
                "p50_ms": Metrics::calc_percentile(&sorted_ttft, 50.0).as_millis(),
                "p90_ms": Metrics::calc_percentile(&sorted_ttft, 90.0).as_millis(),
                "p95_ms": Metrics::calc_percentile(&sorted_ttft, 95.0).as_millis(),
                "p99_ms": Metrics::calc_percentile(&sorted_ttft, 99.0).as_millis()
            },
            "tpot": {
                "min_s": sorted_tpot.first().unwrap_or(&0.0),
                "max_s": sorted_tpot.last().unwrap_or(&0.0),
                "avg_s": if sorted_tpot.is_empty() { 0.0 } else { sorted_tpot.iter().sum::<f64>() / sorted_tpot.len() as f64 },
                "p50_s": calc_percentile_f64(&sorted_tpot, 50.0),
                "p90_s": calc_percentile_f64(&sorted_tpot, 90.0),
                "p95_s": calc_percentile_f64(&sorted_tpot, 95.0),
                "p99_s": calc_percentile_f64(&sorted_tpot, 99.0)
            },
            "tokens": {
                "prompt": {
                    "total": metrics.prompt_tokens.iter().sum::<u64>(),
                    "min": metrics.prompt_tokens.iter().min().unwrap_or(&0),
                    "max": metrics.prompt_tokens.iter().max().unwrap_or(&0),
                    "avg": metrics.prompt_tokens.iter().sum::<u64>() as f64 / metrics.prompt_tokens.len().max(1) as f64,
                    "p50": calc_percentile_u64(&metrics.prompt_tokens, 50.0),
                    "p95": calc_percentile_u64(&metrics.prompt_tokens, 95.0),
                    "per_second": if metrics_elapsed_secs > 0.0 {
                        metrics.prompt_tokens.iter().sum::<u64>() as f64 / metrics_elapsed_secs
                    } else { 0.0 }
                },
                "completion": {
                    "total": metrics.completion_tokens.iter().sum::<u64>(),
                    "min": metrics.completion_tokens.iter().min().unwrap_or(&0),
                    "max": metrics.completion_tokens.iter().max().unwrap_or(&0),
                    "avg": metrics.completion_tokens.iter().sum::<u64>() as f64 / metrics.completion_tokens.len().max(1) as f64,
                    "p50": calc_percentile_u64(&metrics.completion_tokens, 50.0),
                    "p95": calc_percentile_u64(&metrics.completion_tokens, 95.0),
                    "per_second": if metrics_elapsed_secs > 0.0 {
                        metrics.completion_tokens.iter().sum::<u64>() as f64 / metrics_elapsed_secs
                    } else { 0.0 }
                },
                "total": {
                    "total": metrics.total_tokens.iter().sum::<u64>(),
                    "min": metrics.total_tokens.iter().min().unwrap_or(&0),
                    "max": metrics.total_tokens.iter().max().unwrap_or(&0),
                    "avg": metrics.total_tokens.iter().sum::<u64>() as f64 / metrics.total_tokens.len().max(1) as f64,
                    "p50": calc_percentile_u64(&metrics.total_tokens, 50.0),
                    "p95": calc_percentile_u64(&metrics.total_tokens, 95.0),
                    "per_second": if metrics_elapsed_secs > 0.0 {
                        metrics.total_tokens.iter().sum::<u64>() as f64 / metrics_elapsed_secs
                    } else { 0.0 }
                }
            },
            "errors": {
                "count": metrics.errors.len(),
                "rate": if total_completed_requests > 0 {
                    (metrics.errors.len() as f64 / total_completed_requests as f64) * 100.0
                } else { 0.0 },
                "types": &metrics.error_types,
                "oom_occurred": oom_occurred,
                "messages": &metrics.errors
            },
            "timing": {
                "total_time_seconds": metrics.start_time.elapsed().as_secs_f64(),
                "requests_per_second": if metrics_elapsed_secs > 0.0 {
                    total_completed_requests as f64 / metrics_elapsed_secs
                } else { 0.0 },
                "successful_requests": metrics.response_times.len(),
                "failed_requests": metrics.errors.len(),
                "ramp_up_seconds": args.ramp_up_seconds,
                "metrics_collection_seconds": metrics_elapsed_secs,
                "steady_state_seconds": metrics_elapsed_secs,
                "steady_state_requests_per_second": if metrics_elapsed_secs > 0.0 {
                    total_completed_requests as f64 / metrics_elapsed_secs
                } else { 0.0 }
            },
            "per_endpoint": per_endpoint_json
        }
    })
}

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
    mode: MetrumBenchLLMMode,
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
        MetrumBenchLLMMode::Chat => {
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
        MetrumBenchLLMMode::Completion => json!({
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
    metrumbench::banner::print_banner_metrumbench(VERSION, "metrum-ai-bench-llm");
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

    let client = Client::builder()
        .timeout(Duration::from_secs(args.request_timeout))
        .connect_timeout(Duration::from_secs(args.connect_timeout))
        .pool_max_idle_per_host(args.concurrency as usize)
        .pool_idle_timeout(Some(Duration::from_secs(args.pool_idle_timeout)))
        .tcp_keepalive(Some(Duration::from_secs(args.tcp_keepalive)))
        .build()?;

    let mut prompts = load_metrumbench_llm_prompts(&args.prompts)?;

    // Shuffle prompts once for even distribution, then cycle through them deterministically
    let mut shuffle_rng = rand::rngs::StdRng::seed_from_u64(args.common.seed);
    prompts.shuffle(&mut shuffle_rng);
    let mut arrival_rng = rand::rngs::StdRng::seed_from_u64(args.common.seed.wrapping_add(1));
    let slots = metrumbench::load::schedule(
        args.common.arrival_kind(),
        args.num_requests as u64,
        args.common.request_rate.unwrap_or(0.0),
        &mut arrival_rng,
    );

    let sink = Arc::new(
        metrumbench::jsonl::JsonlSink::create(&args.data_log)
            .map_err(|e| format!("Failed to open data log file '{}': {}", args.data_log, e))?,
    );
    let stop = metrumbench::runner::StopFlag::new();
    metrumbench::runner::install_stop_handlers(stop.clone());
    let arrival_kind = args.common.arrival_kind();

    let mut metrics = Metrics::new(args.scenario.clone());
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

    let endpoint_selector = metrumbench::endpoints::EndpointSelector::new(&resolved_endpoints);

    let mut handles = vec![];
    let mut completed = 0;
    let mut last_percentage = 0;

    let start_time = Instant::now();
    let ramp_up_start = start_time;
    let (record_tx, mut record_rx) =
        tokio::sync::mpsc::unbounded_channel::<metrumbench::record::RequestRecord>();

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
        let phase = metrumbench::record::Phase::for_seq(slot.seq, args.common.warmup_requests);

        let permit = semaphore.clone().acquire_owned().await?;
        let ((url, api_key, endpoint_name), endpoint_lease) =
            endpoint_selector.select(&resolved_endpoints, args.common.load_balancer);
        let queue_delay = metrumbench::runner::queue_delay_for_slot(
            arrival_kind,
            start_time.elapsed(),
            slot.scheduled_delay,
        );
        let client = client.clone();
        let prompt = metrumbench::args_common::CommonBenchArgs::unique_prompt(
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
        let record_schedule = metrumbench::runner::should_record_schedule(arrival_kind);
        let run_id_task = run_id.clone();
        let handle = tokio::spawn(async move {
            let started_at = Utc::now();
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

            let tokenizer = match metrumbench::tokenizer::LocalTokenizer::from_file(
                tokenizer_path.as_deref(),
            ) {
                Ok(t) => t,
                Err(_) => metrumbench::tokenizer::LocalTokenizer::from_file(None)
                    .expect("disabled tokenizer"),
            };

            let record = match result {
                Ok(sm) => {
                    let completed_at =
                        metrumbench::runner::completed_at_from_start(started_at, sm.latency);
                    let tokenized_prompt_tokens = tokenizer.count(&sm.prompt_text).ok().flatten();
                    let tokenized_completion_tokens =
                        tokenizer.count(&sm.completion_text).ok().flatten();
                    let usage_missing = sm.completion_tokens == 0 && !sm.completion_text.is_empty();
                    let mut rec = metrumbench::record::RequestRecord::success(
                        seq,
                        phase,
                        endpoint_name.clone(),
                        started_at,
                        completed_at,
                        sm.latency,
                        Some(sm.ttft),
                        sm.first_reasoning,
                        sm.itl,
                        sm.prompt_tokens,
                        sm.completion_tokens,
                        sm.total_tokens,
                    );
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
                    let latency = Utc::now()
                        .signed_duration_since(started_at)
                        .to_std()
                        .unwrap_or(Duration::ZERO);
                    let completed_at =
                        metrumbench::runner::completed_at_from_start(started_at, latency);
                    let mut rec = metrumbench::record::RequestRecord::failed(
                        seq,
                        phase,
                        endpoint_name.clone(),
                        started_at,
                        completed_at,
                        latency,
                        metrumbench::jsonl::classify_error(e.as_ref()),
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
    }
    drop(record_tx);

    info!("All requests launched. Waiting for completion...");

    let mut errors = 0;
    let mut metrics_started = false;
    let mut records: Vec<metrumbench::record::RequestRecord> = Vec::new();

    while let Some(rec) = record_rx.recv().await {
        let endpoint_name = rec.endpoint.clone();
        let phase = rec.phase;
        if rec.is_success() {
            let response_time = Duration::from_secs_f64(rec.latency_s);
            let ttft = Duration::from_secs_f64(rec.ttft_s.unwrap_or(0.0));
            let prompt_tokens = rec.prompt_tokens;
            let completion_tokens = rec.completion_tokens;
            let total_tokens = rec.total_tokens;
            let prompt_words = rec
                .modality_metrics
                .get("prompt_words")
                .copied()
                .unwrap_or(0.0) as usize;
            let completion_words = rec
                .modality_metrics
                .get("completion_words")
                .copied()
                .unwrap_or(0.0) as usize;

            if let Some(ramp_up) = effective_ramp_up {
                if !metrics_started && ramp_up_start.elapsed().as_secs() >= ramp_up {
                    metrics_started = true;
                    metrics.metrics_start_time = Some(Instant::now());
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

            let tpot = if completion_tokens > 1 {
                response_time.checked_sub(ttft).and_then(|gen_time| {
                    if gen_time.is_zero() {
                        None
                    } else {
                        Some(gen_time.as_secs_f64() / (completion_tokens - 1) as f64)
                    }
                })
            } else {
                None
            };
            metrics.record_success(
                &endpoint_name,
                response_time,
                ttft,
                tpot,
                prompt_tokens,
                completion_tokens,
                total_tokens,
                prompt_words,
                completion_words,
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
            if (effective_ramp_up.is_none() || metrics_started)
                && phase != metrumbench::record::Phase::Warmup
            {
                metrics.record_error(&endpoint_name, err_msg);
            }
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
    metrics.print_stats(&resolved_endpoints);

    let window_seconds = metrumbench::runner::window_seconds_from_records(&records);
    let window_seconds = if window_seconds > 0.0 {
        window_seconds
    } else {
        metrics.start_time.elapsed().as_secs_f64()
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
    let mut run_summary = metrumbench::summary::RunSummary::from_records_with_options(
        &records,
        window_seconds,
        stop.is_stopped(),
        &slos,
        args.common.throughput_bin_seconds,
    )
    .with_config(metrumbench::summary::EffectiveRunConfig {
        run_id: run_id.clone(),
        common: (&args.common).into(),
        effective_system_prompt,
        body_template,
        unique_prompt_nonce_template:
            metrumbench::args_common::CommonBenchArgs::unique_prompt_nonce_template(
                args.common.unique_prompts,
            ),
    });
    run_summary.environment =
        metrumbench::environment::collect(ntp_offset_ms, Some(args.model.clone()));
    if let Err(e) = sink.write(&run_summary) {
        warn!("Failed to write summary JSONL: {e}");
    }

    let summary = create_log_record(&args, &metrics, &resolved_endpoints);
    if let Err(e) = sink.write(&summary) {
        return Err(format!("Failed to write to data log: {e}").into());
    }

    // After metrics collection, printing, and logging, check if there were any errors
    if !metrics.errors.is_empty() {
        Err(anyhow::anyhow!("Test completed with errors")
            .context(format!("Total errors: {}", metrics.errors.len()))
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
