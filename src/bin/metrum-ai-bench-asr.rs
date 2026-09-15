// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0
#![allow(clippy::too_many_arguments)]

use chrono::Utc;
use clap::Parser;
use log::{debug, error, info, warn};
use metrumbench::compile_time_info;
use metrumbench::endpoints::{resolve_endpoints, ResolvedEndpoints};
use metrumbench::prompt_inputs::read_utf8_from_path_or_url;
use metrumbench::unique_id;
use rand::SeedableRng;
use reqwest::Client;
use serde_json::{json, Value};
use simplelog::*;
use std::collections::HashMap;
use std::{
    error::Error,
    fs::{self, File},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser, Debug)]
#[command(author, version, about = "A tool for benchmarking audio transcription APIs", long_about = None)]
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
    scenario: Option<String>,

    #[arg(long, help = "URL of the audio transcription API endpoint")]
    url: Option<String>,

    #[arg(long, value_parser = clap::value_parser!(u32).range(1..), help = "Number of requests to send (must be >= 1 when set)")]
    num_requests: Option<u32>,

    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u32).range(1..), help = "Number of concurrent requests (must be >= 1)")]
    concurrency: u32,

    #[arg(
        long,
        help = "Path or http(s) URL to a JSONL file listing audio samples (id, path or url, optional format/duration)"
    )]
    input: Option<String>,

    #[arg(
        long,
        default_value = "info",
        help = "Log level: error, warn, info, debug, trace"
    )]
    log_level: String,

    #[arg(long, help = "Model identifier (e.g., 'whisper-1')")]
    model: Option<String>,

    #[arg(
        long,
        default_value = "results.jsonl",
        help = "Path to the data log file"
    )]
    data_log: String,

    #[command(flatten)]
    common: metrumbench::args_common::CommonBenchArgs,

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

    #[arg(long, help = "API key for authentication")]
    api_key: Option<String>,

    #[arg(
        long,
        help = "Path to YAML file with endpoints (url, api_key, name?, weight?); mutually exclusive with --url/--api-key"
    )]
    endpoints_file: Option<String>,

    #[arg(long, help = "Stop sending new requests after N seconds")]
    stop_after_seconds: Option<u64>,

    #[arg(
        long,
        help = "Path or http(s) URL to a JSONL file with ground-truth transcripts (id, transcript per line)"
    )]
    ground_truth: Option<String>,

    #[arg(
        long,
        default_value = "verbose-json",
        value_enum,
        help = "Response format: verbose_json, json, text, srt, vtt"
    )]
    response_format: MetrumBenchASRResponseFormat,

    #[arg(
        long,
        default_value = "en",
        help = "Language code for transcription (e.g. en, es, fr). Sent in the multipart request to avoid vLLM returning null language in verbose_json responses."
    )]
    language: String,

    #[arg(
        long,
        value_enum,
        default_value_t,
        help = "Text normalization applied to both sides of WER/CER"
    )]
    normalizer: metrumbench::asr::Normalizer,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum MetrumBenchASRResponseFormat {
    VerboseJson,
    Json,
    Text,
    Srt,
    Vtt,
}

impl std::fmt::Display for MetrumBenchASRResponseFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MetrumBenchASRResponseFormat::VerboseJson => write!(f, "verbose_json"),
            MetrumBenchASRResponseFormat::Json => write!(f, "json"),
            MetrumBenchASRResponseFormat::Text => write!(f, "text"),
            MetrumBenchASRResponseFormat::Srt => write!(f, "srt"),
            MetrumBenchASRResponseFormat::Vtt => write!(f, "vtt"),
        }
    }
}

#[derive(Default)]
struct EndpointMetrics {
    response_times: Vec<Duration>,
    inference_times: Vec<Duration>,
    rtf_values: Vec<f64>,
    wer_values: Vec<f64>,
    cer_values: Vec<f64>,
    audio_durations: Vec<f64>,
    characters_per_second: Vec<f64>,
    words_per_second: Vec<f64>,
    errors: Vec<String>,
    total_characters: usize,
    total_words: usize,
    total_requests_completed: usize,
    total_requests_failed: usize,
    total_bytes_sent: usize,
    total_bytes_received: usize,
}

struct Metrics {
    response_times: Vec<Duration>,
    inference_times: Vec<Duration>,
    rtf_values: Vec<f64>,
    wer_values: Vec<f64>,
    cer_values: Vec<f64>,
    audio_durations: Vec<f64>,
    characters_per_second: Vec<f64>,
    words_per_second: Vec<f64>,
    start_time: Instant,
    errors: Vec<String>,
    scenario: String,
    version: String,
    audio_formats: HashMap<String, usize>,
    total_characters: usize,
    total_words: usize,
    total_requests_sent: usize,
    total_requests_completed: usize,
    total_requests_failed: usize,
    total_bytes_sent: usize,
    total_bytes_received: usize,
    request_start_times: Vec<Instant>,
    request_end_times: Vec<Instant>,
    endpoint_metrics: HashMap<String, EndpointMetrics>,
}

impl Metrics {
    fn new(scenario: String) -> Self {
        Self {
            response_times: Vec::new(),
            inference_times: Vec::new(),
            rtf_values: Vec::new(),
            wer_values: Vec::new(),
            cer_values: Vec::new(),
            audio_durations: Vec::new(),
            characters_per_second: Vec::new(),
            words_per_second: Vec::new(),
            start_time: Instant::now(),
            errors: Vec::new(),
            scenario,
            version: VERSION.to_string(),
            audio_formats: HashMap::new(),
            total_characters: 0,
            total_words: 0,
            total_requests_sent: 0,
            total_requests_completed: 0,
            total_requests_failed: 0,
            total_bytes_sent: 0,
            total_bytes_received: 0,
            request_start_times: Vec::new(),
            request_end_times: Vec::new(),
            endpoint_metrics: HashMap::new(),
        }
    }

    fn record_success(
        &mut self,
        endpoint_name: &str,
        response_time: Duration,
        inference_time: Duration,
        bytes_sent: usize,
        bytes_received: usize,
        word_count: usize,
        char_count: usize,
        words_per_second: f64,
        chars_per_second: f64,
        rtf: Option<f64>,
        audio_duration: Option<f64>,
        wer: Option<f64>,
        cer: Option<f64>,
    ) {
        self.response_times.push(response_time);
        self.inference_times.push(inference_time);
        self.total_requests_completed += 1;
        self.total_bytes_sent += bytes_sent;
        self.total_bytes_received += bytes_received;
        self.total_words += word_count;
        self.total_characters += char_count;
        self.words_per_second.push(words_per_second);
        self.characters_per_second.push(chars_per_second);
        if let Some(r) = rtf {
            self.rtf_values.push(r);
        }
        if let Some(d) = audio_duration {
            self.audio_durations.push(d);
        }
        if let Some(w) = wer {
            self.wer_values.push(w);
        }
        if let Some(c) = cer {
            self.cer_values.push(c);
        }
        let ep = self
            .endpoint_metrics
            .entry(endpoint_name.to_string())
            .or_default();
        ep.response_times.push(response_time);
        ep.inference_times.push(inference_time);
        ep.total_requests_completed += 1;
        ep.total_bytes_sent += bytes_sent;
        ep.total_bytes_received += bytes_received;
        ep.total_characters += char_count;
        ep.total_words += word_count;
        ep.words_per_second.push(words_per_second);
        ep.characters_per_second.push(chars_per_second);
        if let Some(r) = rtf {
            ep.rtf_values.push(r);
        }
        if let Some(d) = audio_duration {
            ep.audio_durations.push(d);
        }
        if let Some(w) = wer {
            ep.wer_values.push(w);
        }
        if let Some(c) = cer {
            ep.cer_values.push(c);
        }
    }

    fn record_error(&mut self, endpoint_name: &str, error: String) {
        self.errors.push(error.clone());
        self.total_requests_failed += 1;
        let ep = self
            .endpoint_metrics
            .entry(endpoint_name.to_string())
            .or_default();
        ep.errors.push(error);
        ep.total_requests_failed += 1;
    }

    fn calc_percentile(sorted_values: &[Duration], percentile: f64) -> Duration {
        if sorted_values.is_empty() {
            return Duration::default();
        }
        let index =
            ((sorted_values.len() as f64 * percentile / 100.0).ceil() as usize).saturating_sub(1);
        *sorted_values.get(index).unwrap_or(&Duration::default())
    }

    fn calc_percentile_f64(sorted_values: &[f64], percentile: f64) -> f64 {
        if sorted_values.is_empty() {
            return 0.0;
        }
        let index =
            ((sorted_values.len() as f64 * percentile / 100.0).ceil() as usize).saturating_sub(1);
        *sorted_values.get(index).unwrap_or(&0.0)
    }

    fn print_compact_block(
        &self,
        label: &str,
        response_times: &[Duration],
        inference_times: &[Duration],
        errors: &[String],
        total_chars: usize,
        total_words: usize,
        elapsed_secs: f64,
    ) {
        let requests = response_times.len() + errors.len();
        if elapsed_secs <= 0.0 {
            println!("\n=== {} ===\n  (no timing data)", label);
            return;
        }
        println!("\n=== {} ===", label);
        println!("  Requests:    {}", requests);
        println!("  Errors:      {}", errors.len());
        if !response_times.is_empty() {
            let avg_rt = response_times.iter().sum::<Duration>() / response_times.len() as u32;
            println!("  Avg RT:      {:.3}s", avg_rt.as_secs_f64());
        }
        if !inference_times.is_empty() {
            let avg_inf = inference_times.iter().sum::<Duration>() / inference_times.len() as u32;
            println!("  Avg inference: {:.3}s", avg_inf.as_secs_f64());
        }
        println!("  Chars/words: {} / {}", total_chars, total_words);
        println!("  Req/sec:     {:.1}", requests as f64 / elapsed_secs);
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
                        &ep.inference_times,
                        &ep.errors,
                        ep.total_characters,
                        ep.total_words,
                        elapsed_secs,
                    );
                }
            }
            self.print_compact_block(
                "AGGREGATE",
                &self.response_times,
                &self.inference_times,
                &self.errors,
                self.total_characters,
                self.total_words,
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

        let calc_stats_f64 = |values: &[f64]| {
            if values.is_empty() {
                return (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
            }
            let mut sorted = values.to_vec();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let len = sorted.len();
            let avg = sorted.iter().sum::<f64>() / len.max(1) as f64;
            let p50 = Self::calc_percentile_f64(&sorted, 50.0);
            let p90 = Self::calc_percentile_f64(&sorted, 90.0);
            let p95 = Self::calc_percentile_f64(&sorted, 95.0);
            let p99 = Self::calc_percentile_f64(&sorted, 99.0);
            (
                *sorted.first().unwrap_or(&0.0),
                *sorted.last().unwrap_or(&0.0),
                avg,
                p50,
                p90,
                p95,
                p99,
            )
        };

        println!("\n=====================================");
        println!("MetrumBench ASR Test Results");
        println!("=====================================");
        println!("Scenario: {}", self.scenario);
        println!("Version: {}", self.version);
        println!("Audio Files Processed: {}", self.response_times.len());

        // Calculate total audio duration
        let total_audio_duration: f64 = self.audio_durations.iter().sum();
        println!("Total Audio Duration: {:.1} seconds", total_audio_duration);

        // Print request metrics
        println!("\nRequest Statistics:");
        println!("Total Requests Sent: {}", self.total_requests_sent);
        println!(
            "Total Requests Completed: {}",
            self.total_requests_completed
        );
        println!("Total Requests Failed: {}", self.total_requests_failed);
        println!(
            "Total Bytes Sent: {} MB",
            self.total_bytes_sent as f64 / 1_048_576.0
        );
        println!(
            "Total Bytes Received: {} MB",
            self.total_bytes_received as f64 / 1_048_576.0
        );
        println!(
            "Total Data Transferred: {} MB",
            (self.total_bytes_sent + self.total_bytes_received) as f64 / 1_048_576.0
        );

        let elapsed = self.start_time.elapsed().as_secs_f64();
        println!(
            "Request Rate: {:.2} req/sec",
            self.total_requests_completed as f64 / elapsed
        );
        println!(
            "Data Transfer Rate: {:.2} MB/sec",
            (self.total_bytes_sent + self.total_bytes_received) as f64 / 1_048_576.0 / elapsed
        );

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

            let (inf_min, inf_max, inf_avg, inf_p50, inf_p90, inf_p95, inf_p99) =
                calc_stats(&self.inference_times);
            println!("\nInference Time Statistics:");
            println!("Min: {:?}, Max: {:?}, Avg: {:?}", inf_min, inf_max, inf_avg);
            println!(
                "p50: {:?}, p90: {:?}, p95: {:?}, p99: {:?}",
                inf_p50, inf_p90, inf_p95, inf_p99
            );
        } else {
            println!("\nNo successful responses to calculate timing statistics.");
        }

        // Print RTF stats if available
        if !self.rtf_values.is_empty() {
            let (rtf_min, rtf_max, rtf_avg, rtf_p50, rtf_p90, rtf_p95, rtf_p99) =
                calc_stats_f64(&self.rtf_values);
            println!("\nReal-Time Factor (RTF) Statistics:");
            println!(
                "Min: {:.2}, Max: {:.2}, Avg: {:.2}",
                rtf_min, rtf_max, rtf_avg
            );
            println!(
                "p50: {:.2}, p90: {:.2}, p95: {:.2}, p99: {:.2}",
                rtf_p50, rtf_p90, rtf_p95, rtf_p99
            );
        }

        // Print WER stats if available
        if !self.wer_values.is_empty() {
            let (wer_min, wer_max, wer_avg, wer_p50, wer_p90, wer_p95, wer_p99) =
                calc_stats_f64(&self.wer_values);
            println!("\nWord Error Rate (WER) Statistics:");
            println!(
                "Min: {:.1}%, Max: {:.1}%, Avg: {:.1}%",
                wer_min * 100.0,
                wer_max * 100.0,
                wer_avg * 100.0
            );
            println!(
                "p50: {:.1}%, p90: {:.1}%, p95: {:.1}%, p99: {:.1}%",
                wer_p50 * 100.0,
                wer_p90 * 100.0,
                wer_p95 * 100.0,
                wer_p99 * 100.0
            );
        }

        // Print CER stats if available
        if !self.cer_values.is_empty() {
            let (cer_min, cer_max, cer_avg, cer_p50, cer_p90, cer_p95, cer_p99) =
                calc_stats_f64(&self.cer_values);
            println!("\nCharacter Error Rate (CER) Statistics:");
            println!(
                "Min: {:.1}%, Max: {:.1}%, Avg: {:.1}%",
                cer_min * 100.0,
                cer_max * 100.0,
                cer_avg * 100.0
            );
            println!(
                "p50: {:.1}%, p90: {:.1}%, p95: {:.1}%, p99: {:.1}%",
                cer_p50 * 100.0,
                cer_p90 * 100.0,
                cer_p95 * 100.0,
                cer_p99 * 100.0
            );
        }

        // Print throughput statistics
        if !self.response_times.is_empty() {
            println!("\nThroughput Statistics:");

            // Audio processed per second
            let audio_per_second = total_audio_duration / self.start_time.elapsed().as_secs_f64();
            println!(
                "Avg Audio Processed/Second: {:.2} sec/sec",
                audio_per_second
            );

            // Characters per second
            if !self.characters_per_second.is_empty() {
                let avg_cps = self.characters_per_second.iter().sum::<f64>()
                    / self.characters_per_second.len() as f64;
                println!("Avg Characters/Second: {:.2} chars/sec", avg_cps);
            }

            // Words per second
            if !self.words_per_second.is_empty() {
                let avg_wps =
                    self.words_per_second.iter().sum::<f64>() / self.words_per_second.len() as f64;
                println!("Avg Words/Second: {:.2} words/sec", avg_wps);
            }

            // Requests per second
            let rps = self.response_times.len() as f64 / self.start_time.elapsed().as_secs_f64();
            println!("Avg Requests/Second: {:.2} RPS", rps);
        }

        // Add error statistics
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

        // Audio format statistics
        if !self.audio_formats.is_empty() {
            println!("\nAudio Format Statistics:");
            for (format, count) in self.audio_formats.iter() {
                println!("  {}: {} files", format, count);
            }
        }

        // Test duration
        println!(
            "\nTest Duration: {:.2} seconds",
            self.start_time.elapsed().as_secs_f64()
        );
    }
}

fn word_error_rate(
    reference: &str,
    hypothesis: &str,
    normalizer: metrumbench::asr::Normalizer,
) -> f64 {
    metrumbench::asr::word_error_rate(reference, hypothesis, normalizer).unwrap_or(1.0)
}

fn character_error_rate(
    reference: &str,
    hypothesis: &str,
    normalizer: metrumbench::asr::Normalizer,
) -> f64 {
    metrumbench::asr::character_error_rate(reference, hypothesis, normalizer).unwrap_or(1.0)
}

#[derive(Clone)]
struct AudioSample {
    id: String,
    url: Option<String>, // URL is now optional if we have a direct path
    format: String,
    duration: Option<f64>,
    ground_truth: Option<String>,
    // Stores the local file path (either from direct path or after download)
    local_file_path: Option<String>,
}

async fn make_request(
    client: &Client,
    url: &str,
    model: &str,
    audio_sample: &AudioSample,
    request_timeout: u64,
    api_key: &str,
    response_format: &str,
    language: &str,
) -> Result<
    (Duration, Duration, String, f64, &'static str, usize, usize),
    Box<dyn Error + Send + Sync>,
> {
    let local_file_path = match &audio_sample.local_file_path {
        Some(path) => path,
        None => return Err("No local file path available for audio sample".into()),
    };

    debug!("Request URL: {}", url);
    debug!("Using local file: {}", local_file_path);
    debug!("Model: {}", model);
    debug!("Response format: {}", response_format);

    // Read the file content before starting the request clock.
    let file_content = tokio::fs::read(local_file_path).await?;
    let content_size = file_content.len();
    debug!("File size: {} bytes", content_size);
    let start_time = Instant::now();

    // Use basename only for multipart filename (do not leak full path); MIME from format
    let basename = Path::new(local_file_path)
        .file_name()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("audio.{}", audio_sample.format));
    let mime = mime_for_format(&audio_sample.format);
    let file_part = reqwest::multipart::Part::bytes(file_content)
        .file_name(basename)
        .mime_str(mime)?;

    // Start building the form
    let mut form = reqwest::multipart::Form::new()
        .part("file", file_part)
        .text("model", model.to_string())
        .text("response_format", response_format.to_string());

    if !language.is_empty() {
        form = form.text("language", language.to_string());
    }

    // Only add timestamp_granularities for json/verbose_json formats
    if response_format == "json" || response_format == "verbose_json" {
        form = form.text("timestamp_granularities[]", "word");
    }

    let response = match client
        .post(url)
        .header("Authorization", format!("Bearer {}", api_key))
        .multipart(form)
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
        let error_body = response
            .text()
            .await
            .unwrap_or_else(|_| "No error body".to_string());
        warn!(
            "Request failed with status: {} - Body: {}",
            status, error_body
        );
        return Err(metrumbench::error::RequestError::from_status(status.as_u16()).into());
    }

    // Get the response text and track bytes received
    let response_text = response.text().await?;
    let response_size = response_text.len();
    debug!("Response body size: {} bytes", response_size);

    // Parse response based on format
    let (transcription, inference_time, inference_time_source) = match response_format {
        "verbose_json" | "json" => {
            let json_resp: Value = serde_json::from_str(&response_text)?;
            let text = if let Some(text) = json_resp.get("text").and_then(|v| v.as_str()) {
                text.to_string()
            } else if let Some(text) = json_resp.get("transcription").and_then(|v| v.as_str()) {
                text.to_string()
            } else {
                return Err("No transcription text in response".into());
            };

            let raw = json_resp.get("inference_time").and_then(|v| v.as_f64());
            let (inference_time, source) = match raw {
                Some(t) if t.is_finite() && t >= 0.0 => (t, "server"),
                _ => (start_time.elapsed().as_secs_f64(), "client"),
            };
            (text, inference_time, source)
        }
        "text" | "srt" | "vtt" => (response_text, start_time.elapsed().as_secs_f64(), "client"),
        _ => return Err(format!("Unsupported response format: {}", response_format).into()),
    };

    let total_time = start_time.elapsed();

    Ok((
        total_time,
        first_byte,
        transcription,
        inference_time,
        inference_time_source,
        content_size,
        response_size,
    ))
}

fn mime_for_format(format: &str) -> &'static str {
    match format.to_lowercase().as_str() {
        "mp3" | "mpeg" => "audio/mpeg",
        "wav" => "audio/wav",
        "webm" => "audio/webm",
        "ogg" | "oga" => "audio/ogg",
        "m4a" | "mp4" => "audio/mp4",
        "flac" => "audio/flac",
        _ => "audio/mpeg",
    }
}

/// Collision-safe cache key from URL so different URLs never share the same file.
fn url_cache_key(url: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut h);
    h.finish()
}

// Function to download an audio file from a URL
async fn download_audio_file(
    client: &Client,
    url: &str,
    format: &str,
) -> Result<String, Box<dyn Error + Send + Sync>> {
    let temp_dir = PathBuf::from("audio");
    if !temp_dir.exists() {
        fs::create_dir_all(&temp_dir)?;
    }
    // Use URL-based cache key to avoid collisions when different URLs have the same basename
    let file_name = format!("{:016x}.{}", url_cache_key(url), format);
    let file_path = temp_dir.join(&file_name);
    let file_path_str = file_path.to_string_lossy().into_owned();

    // Check if file already exists
    if file_path.exists() {
        let metadata = fs::metadata(&file_path)?;
        info!(
            "Using existing audio file at {} (size: {} bytes)",
            file_path.display(),
            metadata.len()
        );
        return Ok(file_path_str);
    }

    info!(
        "Downloading audio file from {} to {}",
        url,
        file_path.display()
    );

    // Download the file
    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        return Err(format!("Failed to download file: HTTP status {}", response.status()).into());
    }

    // Save the file
    let content = response.bytes().await?;
    fs::write(&file_path, content)?;

    // Verify file was written successfully
    let metadata = fs::metadata(&file_path)?;
    info!(
        "Successfully downloaded audio file to {} (size: {} bytes)",
        file_path.display(),
        metadata.len()
    );

    Ok(file_path_str)
}

fn load_audio_samples(input_path: &str) -> Result<Vec<AudioSample>, Box<dyn Error + Send + Sync>> {
    let content = read_utf8_from_path_or_url(input_path)?;
    let mut samples = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let sample: Value = serde_json::from_str(line)?;

        let id = sample
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or("Missing id field in audio sample")?
            .to_string();

        // Check if we have a direct path first, then fall back to URL
        let path = sample
            .get("path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let url = sample
            .get("url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Ensure we have either a path or URL
        if path.is_none() && url.is_none() {
            return Err("Audio sample must have either 'path' or 'url' field".into());
        }

        let format = sample
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let duration = sample.get("duration").and_then(|v| v.as_f64());

        // If we have a direct path, use it as the local_file_path
        let local_file_path = path;

        samples.push(AudioSample {
            id,
            url,
            format,
            duration,
            ground_truth: None,
            local_file_path,
        });
    }

    Ok(samples)
}

fn load_ground_truth(path: &str) -> Result<HashMap<String, String>, Box<dyn Error + Send + Sync>> {
    let content = read_utf8_from_path_or_url(path)?;
    let mut ground_truth = HashMap::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let record: Value = serde_json::from_str(line)?;

        let id = record
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or("Missing id field in ground truth")?
            .to_string();

        let transcript = record
            .get("transcript")
            .and_then(|v| v.as_str())
            .ok_or("Missing transcript field in ground truth")?
            .to_string();

        ground_truth.insert(id, transcript);
    }

    Ok(ground_truth)
}

fn create_log_record(args: &Args, metrics: &Metrics, resolved: &ResolvedEndpoints) -> Value {
    let mut sorted_rt = metrics.response_times.clone();
    sorted_rt.sort();
    let mut sorted_inf = metrics.inference_times.clone();
    sorted_inf.sort();
    let mut sorted_rtf = metrics.rtf_values.clone();
    sorted_rtf.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut sorted_wer = metrics.wer_values.clone();
    sorted_wer.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut sorted_cer = metrics.cer_values.clone();
    sorted_cer.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

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
                    "total_characters": ep.total_characters,
                    "total_words": ep.total_words,
                    "requests_per_second": if ep_elapsed > 0.0 { ep_requests as f64 / ep_elapsed } else { 0.0 }
                }),
            )
        })
        .collect();

    json!({
        "unique_id": unique_id::generate_uuid(),
        "human_readable_id": unique_id::generate_human_readable_unique_id(3),
        "timestamp": Utc::now().to_rfc3339(),
        "metrumbench_asr_version": VERSION,
        "compile_info": compile_time_info::get_compile_info(),
        "config": {
            "scenario": args.scenario.as_ref().unwrap(),
            "url": config_url,
            "endpoint": config_endpoint,
            "endpoints": config_endpoints,
            "model": args.model.as_ref().unwrap(),
            "num_requests": args.num_requests.unwrap(),
            "concurrency": args.concurrency,
            "log_level": args.log_level,
            "input_file": args.input.as_ref().unwrap(),
            "data_log": args.data_log,
            "debug_log": args.debug_log,
            "error_log": args.error_log,
            "request_timeout": args.request_timeout,
            "connect_timeout": args.connect_timeout,
            "pool_idle_timeout": args.pool_idle_timeout,
            "tcp_keepalive": args.tcp_keepalive,
            "stop_after_seconds": args.stop_after_seconds,
            "language": args.language,
            "ground_truth": args.ground_truth,
            "response_format": format!("{}", args.response_format),
            "normalizer": format!("{}", args.normalizer),
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
            "inference_times": {
                "min_ms": sorted_inf.first().unwrap_or(&Duration::default()).as_millis(),
                "max_ms": sorted_inf.last().unwrap_or(&Duration::default()).as_millis(),
                "avg_ms": (sorted_inf.iter().sum::<Duration>() / sorted_inf.len().max(1) as u32).as_millis(),
                "p50_ms": Metrics::calc_percentile(&sorted_inf, 50.0).as_millis(),
                "p90_ms": Metrics::calc_percentile(&sorted_inf, 90.0).as_millis(),
                "p95_ms": Metrics::calc_percentile(&sorted_inf, 95.0).as_millis(),
                "p99_ms": Metrics::calc_percentile(&sorted_inf, 99.0).as_millis()
            },
            "rtf": {
                "min": metrics.rtf_values.iter().copied().min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)).unwrap_or(0.0),
                "max": metrics.rtf_values.iter().copied().max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)).unwrap_or(0.0),
                "avg": metrics.rtf_values.iter().sum::<f64>() / metrics.rtf_values.len().max(1) as f64,
                "p50": Metrics::calc_percentile_f64(&sorted_rtf, 50.0),
                "p90": Metrics::calc_percentile_f64(&sorted_rtf, 90.0),
                "p95": Metrics::calc_percentile_f64(&sorted_rtf, 95.0),
                "p99": Metrics::calc_percentile_f64(&sorted_rtf, 99.0)
            },
            "accuracy": {
                "wer": {
                    "min": metrics.wer_values.iter().copied().min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)).unwrap_or(0.0),
                    "max": metrics.wer_values.iter().copied().max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)).unwrap_or(0.0),
                    "avg": metrics.wer_values.iter().sum::<f64>() / metrics.wer_values.len().max(1) as f64,
                    "p50": Metrics::calc_percentile_f64(&sorted_wer, 50.0),
                    "p90": Metrics::calc_percentile_f64(&sorted_wer, 90.0),
                    "p95": Metrics::calc_percentile_f64(&sorted_wer, 95.0),
                    "p99": Metrics::calc_percentile_f64(&sorted_wer, 99.0)
                },
                "cer": {
                    "min": metrics.cer_values.iter().copied().min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)).unwrap_or(0.0),
                    "max": metrics.cer_values.iter().copied().max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)).unwrap_or(0.0),
                    "avg": metrics.cer_values.iter().sum::<f64>() / metrics.cer_values.len().max(1) as f64,
                    "p50": Metrics::calc_percentile_f64(&sorted_cer, 50.0),
                    "p90": Metrics::calc_percentile_f64(&sorted_cer, 90.0),
                    "p95": Metrics::calc_percentile_f64(&sorted_cer, 95.0),
                    "p99": Metrics::calc_percentile_f64(&sorted_cer, 99.0)
                }
            },
            "throughput": {
                "total_audio_seconds": metrics.audio_durations.iter().sum::<f64>(),
                "total_characters": metrics.total_characters,
                "total_words": metrics.total_words,
                "audio_per_second": metrics.audio_durations.iter().sum::<f64>() / metrics.start_time.elapsed().as_secs_f64(),
                "characters_per_second": metrics.total_characters as f64 / metrics.start_time.elapsed().as_secs_f64(),
                "words_per_second": metrics.total_words as f64 / metrics.start_time.elapsed().as_secs_f64(),
                "requests_per_second": metrics.response_times.len() as f64 / metrics.start_time.elapsed().as_secs_f64()
            },
            "errors": {
                "count": metrics.errors.len(),
                "rate": (metrics.errors.len() as f64 / (metrics.response_times.len() + metrics.errors.len()) as f64) * 100.0
            },
            "timing": {
                "total_time_seconds": metrics.start_time.elapsed().as_secs_f64(),
                "successful_requests": metrics.response_times.len(),
                "failed_requests": metrics.errors.len()
            },
            "request_metrics": {
                "total_requests": {
                    "sent": metrics.total_requests_sent,
                    "completed": metrics.total_requests_completed,
                    "failed": metrics.total_requests_failed
                },
                "bytes": {
                    "sent": metrics.total_bytes_sent,
                    "received": metrics.total_bytes_received,
                    "total": metrics.total_bytes_sent + metrics.total_bytes_received
                },
                "request_rate": {
                    "requests_per_second": metrics.total_requests_completed as f64 / metrics.start_time.elapsed().as_secs_f64(),
                    "bytes_per_second": (metrics.total_bytes_sent + metrics.total_bytes_received) as f64 / metrics.start_time.elapsed().as_secs_f64()
                }
            },
            "audio_formats": metrics.audio_formats,
            "per_endpoint": per_endpoint_json
        }
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    // Print banner
    metrumbench::banner::print_banner_metrumbench(VERSION, "metrum-ai-bench-asr");

    let args = Args::parse();

    // Check for version-only flag first
    if args.version_only {
        println!("metrum-ai-bench-asr version {}", VERSION);
        return Ok(());
    }

    // Required: scenario, num_requests, input, model
    if args.scenario.is_none()
        || args.num_requests.is_none()
        || args.input.is_none()
        || args.model.is_none()
    {
        eprintln!("Error: When not using --version-only, the following arguments are required:");
        eprintln!("  --scenario, --num-requests, --input, --model");
        return Err("Missing required arguments".into());
    }
    // Endpoint source: exactly one of (--url + --api-key) or --endpoints-file
    let resolved_endpoints = resolve_endpoints(
        args.url.as_deref(),
        args.api_key.as_deref(),
        args.endpoints_file.as_deref(),
    )
    .map_err(|e| {
        eprintln!("Error: {}", e);
        e
    })?;
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
        _ => LevelFilter::Info,
    };

    // Create log directory if it doesn't exist
    if let Some(parent) = Path::new(&args.debug_log).parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }
    if let Some(parent) = Path::new(&args.error_log).parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }
    if let Some(parent) = Path::new(&args.data_log).parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }

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

    let num_requests = args.num_requests.unwrap();
    info!(
        "Starting audio transcription benchmark with {} requests at {} concurrency",
        num_requests, args.concurrency
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

    // Load audio samples from input JSONL file
    let mut audio_samples = load_audio_samples(args.input.as_ref().unwrap())?;
    info!(
        "Loaded {} audio samples from '{}'",
        audio_samples.len(),
        args.input.as_ref().unwrap()
    );

    // Download audio files from URLs if needed; per-sample failures are recorded, not fatal
    info!("Preparing audio files...");
    let mut metrics = Metrics::new(args.scenario.as_ref().unwrap().clone());
    for sample in &mut audio_samples {
        if sample.local_file_path.is_some() {
            let path = sample.local_file_path.as_ref().unwrap();
            if !std::path::Path::new(path).exists() {
                metrics.record_error(
                    "unknown",
                    format!("Local file not found for sample {}: {}", sample.id, path),
                );
                sample.local_file_path = None;
                continue;
            }
            let metadata = std::fs::metadata(path)?;
            info!(
                "Using local audio file for sample {}: {} (size: {} bytes)",
                sample.id,
                path,
                metadata.len()
            );
            continue;
        }
        if let Some(url) = &sample.url {
            match download_audio_file(&client, url, &sample.format).await {
                Ok(local_path) => {
                    sample.local_file_path = Some(local_path);
                    info!("Downloaded audio file for sample {}", sample.id);
                }
                Err(e) => {
                    metrics.record_error("unknown", format!("Sample {}: {}", sample.id, e));
                    sample.local_file_path = None;
                }
            }
        } else {
            metrics.record_error(
                "unknown",
                format!("Sample {} has neither URL nor local path", sample.id),
            );
            sample.local_file_path = None;
        }
    }
    let mut audio_samples: Vec<AudioSample> = audio_samples
        .into_iter()
        .filter(|s| s.local_file_path.is_some())
        .collect();
    if audio_samples.is_empty() {
        return Err(
            "No audio samples available after preparation (all downloads or paths failed)".into(),
        );
    }
    info!("All audio files ready ({} samples)", audio_samples.len());

    // Load ground truth transcriptions if provided
    let ground_truth = if let Some(gt_path) = &args.ground_truth {
        info!("Loading ground truth transcriptions from '{}'", gt_path);
        let gt = load_ground_truth(gt_path)?;
        info!("Loaded {} ground truth transcriptions", gt.len());

        for sample in &mut audio_samples {
            if let Some(transcript) = gt.get(&sample.id) {
                sample.ground_truth = Some(transcript.clone());
            }
        }
        Some(gt)
    } else {
        None
    };

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

    // Total number of requests to send
    let num_requests = args.num_requests.unwrap();
    // Must have at least one audio sample
    if audio_samples.is_empty() {
        return Err("No audio samples to process".into());
    }

    let start_time = Instant::now();
    let (record_tx, mut record_rx) =
        tokio::sync::mpsc::unbounded_channel::<metrumbench::record::RequestRecord>();
    let mut arrival_rng = rand::rngs::StdRng::seed_from_u64(args.common.seed);
    let slots = metrumbench::load::schedule(
        arrival_kind,
        u64::from(num_requests),
        args.common.request_rate.unwrap_or(0.0),
        &mut arrival_rng,
    );
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

        let delay = slot.scheduled_delay.saturating_sub(start_time.elapsed());
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        let permit = semaphore.clone().acquire_owned().await?;
        let client = client.clone();
        let queue_delay = metrumbench::runner::queue_delay_for_slot(
            arrival_kind,
            start_time.elapsed(),
            slot.scheduled_delay,
        );
        let i = slot.seq as u32;

        // Get the next audio sample (round-robin if fewer samples than requests)
        let sample = audio_samples[(i as usize) % audio_samples.len()].clone();

        // Track audio format
        *metrics
            .audio_formats
            .entry(sample.format.clone())
            .or_insert(0) += 1;

        let ((url, api_key, endpoint_name), endpoint_lease) =
            endpoint_selector.select(&resolved_endpoints, args.common.load_balancer);
        let endpoint_selector = endpoint_selector.clone();
        let model = args.model.as_ref().unwrap().clone();
        let request_timeout = args.request_timeout;
        let response_format_str = format!("{}", args.response_format);
        let language_str = args.language.clone();
        let ground_truth_sample = if let Some(gt) = &sample.ground_truth {
            Some(gt.clone())
        } else {
            ground_truth
                .as_ref()
                .and_then(|gt| gt.get(&sample.id).cloned())
        };

        let sample_duration = sample.duration;
        let sample_id = sample.id.clone();
        let phase = metrumbench::record::Phase::for_seq(slot.seq, args.common.warmup_requests);
        let scheduled_delay = slot.scheduled_delay;
        let record_schedule = metrumbench::runner::should_record_schedule(arrival_kind);
        let normalizer = args.normalizer;
        let sink_task = sink.clone();
        let record_tx = record_tx.clone();
        let run_id_task = run_id.clone();

        // Track request start
        metrics.total_requests_sent += 1;
        metrics.request_start_times.push(Instant::now());

        let handle = tokio::spawn(async move {
            let started_at = Utc::now();
            let result = make_request(
                &client,
                &url,
                &model,
                &sample,
                request_timeout,
                &api_key,
                &response_format_str,
                &language_str,
            )
            .await;
            drop(permit);
            drop(endpoint_lease);

            let record = match result {
                Ok((
                    response_time,
                    first_byte,
                    transcription,
                    inference_time,
                    inference_time_source,
                    bytes_sent,
                    bytes_received,
                )) => {
                    let completed_at =
                        metrumbench::runner::completed_at_from_start(started_at, response_time);
                    let word_count = transcription.split_whitespace().count();
                    let char_count = transcription.chars().count();
                    let words_per_second = if inference_time > 0.0 {
                        word_count as f64 / inference_time
                    } else {
                        0.0
                    };
                    let chars_per_second = if inference_time > 0.0 {
                        char_count as f64 / inference_time
                    } else {
                        0.0
                    };
                    let (rtf, audio_duration) = sample_duration
                        .filter(|duration| *duration > 0.0)
                        .map(|duration| (inference_time / duration, duration))
                        .unzip();
                    let rtfx = sample_duration
                        .and_then(|d| metrumbench::asr::rtfx(d, response_time.as_secs_f64()));
                    let (wer, cer) = ground_truth_sample
                        .as_ref()
                        .map(|gt| {
                            (
                                word_error_rate(gt, &transcription, normalizer),
                                character_error_rate(gt, &transcription, normalizer),
                            )
                        })
                        .unzip();
                    let mut rec = metrumbench::record::RequestRecord::success(
                        slot.seq,
                        phase,
                        endpoint_name.clone(),
                        started_at,
                        completed_at,
                        response_time,
                        None,
                        None,
                        Vec::new(),
                        0,
                        0,
                        0,
                    )
                    .with_first_byte(first_byte);
                    if record_schedule {
                        rec = rec.with_schedule(scheduled_delay, queue_delay);
                    }
                    if let Some(value) = rtfx {
                        rec.modality_metrics.insert("rtfx_client".into(), value);
                    }
                    if let Some(value) = wer {
                        rec.modality_metrics.insert("wer".into(), value);
                    }
                    if let Some(value) = cer {
                        rec.modality_metrics.insert("cer".into(), value);
                    }
                    if let Some(value) = rtf {
                        rec.modality_metrics.insert("rtf".into(), value);
                    }
                    if let Some(value) = audio_duration {
                        rec.modality_metrics
                            .insert("audio_duration_s".into(), value);
                    }
                    rec.modality_metrics
                        .insert("inference_time_s".into(), inference_time);
                    rec.modality_metrics
                        .insert("word_count".into(), word_count as f64);
                    rec.modality_metrics
                        .insert("char_count".into(), char_count as f64);
                    rec.modality_metrics
                        .insert("words_per_second".into(), words_per_second);
                    rec.modality_metrics
                        .insert("chars_per_second".into(), chars_per_second);
                    rec.modality_metrics
                        .insert("bytes_sent".into(), bytes_sent as f64);
                    rec.modality_metrics
                        .insert("bytes_received".into(), bytes_received as f64);
                    rec.modality_metrics.insert(
                        format!("inference_seconds_{inference_time_source}"),
                        inference_time,
                    );
                    if let Some(audio_duration) = sample_duration {
                        debug!(
                            "RTF for sample {}: {:.2} (duration: {:.2}s, inference: {:.2}s)",
                            sample_id,
                            inference_time / audio_duration,
                            audio_duration,
                            inference_time
                        );
                    }
                    if ground_truth_sample.is_some() {
                        debug!(
                            "Accuracy for sample {}: WER={:.2}%, CER={:.2}%",
                            sample_id,
                            wer.unwrap_or(0.0) * 100.0,
                            cer.unwrap_or(0.0) * 100.0
                        );
                    }
                    rec.with_run_id(run_id_task)
                }
                Err(e) => {
                    error!("Request failed for sample {}: {}", sample_id, e);
                    if let Some(source) = std::error::Error::source(&*e) {
                        error!("Caused by: {}", source);
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
                        slot.seq,
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
    }
    drop(record_tx);

    info!("All requests launched. Waiting for completion...");

    let mut errors = 0;
    let mut records = Vec::new();
    while let Some(rec) = record_rx.recv().await {
        let endpoint_name = rec.endpoint.clone();
        let phase = rec.phase;
        if rec.is_success() {
            let response_time = Duration::from_secs_f64(rec.latency_s);
            let inference_time = rec
                .modality_metrics
                .get("inference_time_s")
                .copied()
                .unwrap_or(0.0);
            let inference_time_duration = Duration::from_secs_f64(inference_time);
            let word_count = rec
                .modality_metrics
                .get("word_count")
                .copied()
                .unwrap_or(0.0) as usize;
            let char_count = rec
                .modality_metrics
                .get("char_count")
                .copied()
                .unwrap_or(0.0) as usize;
            let words_per_second = rec
                .modality_metrics
                .get("words_per_second")
                .copied()
                .unwrap_or(0.0);
            let chars_per_second = rec
                .modality_metrics
                .get("chars_per_second")
                .copied()
                .unwrap_or(0.0);
            let bytes_sent = rec
                .modality_metrics
                .get("bytes_sent")
                .copied()
                .unwrap_or(0.0) as usize;
            let bytes_received = rec
                .modality_metrics
                .get("bytes_received")
                .copied()
                .unwrap_or(0.0) as usize;
            let rtf = rec.modality_metrics.get("rtf").copied();
            let audio_duration = rec.modality_metrics.get("audio_duration_s").copied();
            let wer = rec.modality_metrics.get("wer").copied();
            let cer = rec.modality_metrics.get("cer").copied();

            if phase == metrumbench::record::Phase::Warmup {
                records.push(rec);
                continue;
            }

            metrics.record_success(
                &endpoint_name,
                response_time,
                inference_time_duration,
                bytes_sent,
                bytes_received,
                word_count,
                char_count,
                words_per_second,
                chars_per_second,
                rtf,
                audio_duration,
                wer,
                cer,
            );
            metrics.request_end_times.push(Instant::now());
            completed += 1;
            debug!(
                "Request completed - RT: {:?}, Words: {}, Chars: {}",
                response_time, word_count, char_count
            );
            let percentage = (completed * 100) / (num_requests as usize);
            if percentage >= last_percentage + 10 {
                info!(
                    "{}% complete ({}/{} requests)",
                    percentage, completed, num_requests
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
            if phase != metrumbench::record::Phase::Warmup {
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
        completed, num_requests, errors
    );
    metrics.print_stats(&resolved_endpoints);
    let window_seconds = metrumbench::runner::window_seconds_from_records(&records);
    let window_seconds = if window_seconds > 0.0 {
        window_seconds
    } else {
        start_time.elapsed().as_secs_f64()
    };
    let slos = args.common.parse_slos()?;
    let body_template = json!({
        "multipart_fields": ["file", "model", "response_format", "language", "timestamp_granularities[]"],
        "model": args.model,
        "response_format": format!("{}", args.response_format),
        "language": args.language,
        "timestamp_granularities": ["word"],
        "file": "<redacted audio bytes>"
    });
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
        effective_system_prompt: None,
        body_template,
        unique_prompt_nonce_template: None,
    });
    shared_summary.environment =
        metrumbench::environment::collect(ntp_offset_ms, args.model.clone());
    if let Err(e) = sink.write(&shared_summary) {
        warn!("Failed to write summary JSONL: {e}");
    }

    let summary = create_log_record(&args, &metrics, &resolved_endpoints);
    if let Err(e) = sink.write(&summary) {
        return Err(format!("Failed to write to data log: {e}").into());
    }

    // Clean up temporary files
    info!("Cleaning up temporary audio files...");
    if let Err(e) = cleanup_temp_files() {
        warn!("Failed to clean up temporary files: {}", e);
    }

    // Function to clean up temporary files
    fn cleanup_temp_files() -> Result<(), Box<dyn Error + Send + Sync>> {
        // Note: We're not removing the "audio" directory anymore since it's now part of the test fixtures
        // If needed, we can clear specific files but keep the directory

        // For now, we'll just log that we're keeping the files for debugging
        info!("Audio files kept in the audio directory for future testing");
        Ok(())
    }

    if args.common.fail_on_error && !metrics.errors.is_empty() {
        Err("Test completed with errors".into())
    } else {
        Ok(())
    }
}
