// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Strategic benchmark primitives: sweeps, validity, server correlation and exports.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchRecord {
    pub seq: u64,
    pub stage: f64,
    pub endpoint: String,
    pub scheduled_unix_ns: u128,
    pub sent_unix_ns: u128,
    /// Scheduled-to-completion latency. This includes client queueing delay.
    pub latency_s: f64,
    /// Scheduled-to-send delay, exposing coordinated omission under overload.
    pub queue_delay_s: f64,
    /// Send-to-completion service latency.
    pub service_latency_s: f64,
    /// Send-to-response-headers timing, matching the chat benchmark binaries.
    #[serde(default)]
    pub first_byte_s: Option<f64>,
    /// Connector TCP/TLS duration; `0.0` is a pool hit.
    #[serde(default)]
    pub connect_s: Option<f64>,
    /// Send-to-first-visible-output timing; absent for unary responses.
    #[serde(default)]
    pub ttft_s: Option<f64>,
    /// Prefill proxy (`ttft - connect` or `ttft`).
    #[serde(default)]
    pub prefill_s: Option<f64>,
    /// Decode proxy (`service_latency - ttft`).
    #[serde(default)]
    pub decode_s: Option<f64>,
    /// Output tokens per decode second.
    #[serde(default)]
    pub decode_tok_s: Option<f64>,
    /// Inter-token latency samples from streaming visible-output chunks.
    /// Serialized as a semicolon-joined string for CSV compatibility.
    #[serde(
        default,
        serialize_with = "serialize_itl_s",
        deserialize_with = "deserialize_itl_s"
    )]
    pub itl_s: Vec<f64>,
    /// Outstanding client requests at send (after semaphore acquire).
    #[serde(default)]
    pub in_flight_at_send: Option<u64>,
    pub success: bool,
    pub valid: Option<bool>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub session_id: Option<String>,
    pub turn: Option<usize>,
    pub error: Option<String>,
    /// Warmup requests are retained for audit but excluded from stage aggregates.
    #[serde(default)]
    pub warmup: bool,
}

fn serialize_itl_s<S>(itl: &[f64], serializer: S) -> std::result::Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let joined = itl
        .iter()
        .map(|value| format!("{value}"))
        .collect::<Vec<_>>()
        .join(";");
    serializer.serialize_str(&joined)
}

fn deserialize_itl_s<'de, D>(deserializer: D) -> std::result::Result<Vec<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    raw.split(';')
        .map(|part| {
            part.parse::<f64>()
                .map_err(|err| serde::de::Error::custom(format!("invalid itl_s sample: {err}")))
        })
        .collect()
}

impl BenchRecord {
    /// N-1 TPOT: `(service_latency - ttft) / (output_tokens - 1)`.
    pub fn tpot_s(&self) -> Option<f64> {
        let ttft = self.ttft_s?;
        let gen = (self.service_latency_s - ttft).max(0.0);
        if self.output_tokens <= 1 || gen == 0.0 {
            return None;
        }
        Some(gen / (self.output_tokens - 1) as f64)
    }

    /// Output tokens per second for this in-flight user/stream.
    pub fn user_tps(&self) -> Option<f64> {
        if !self.success || self.output_tokens == 0 || self.service_latency_s <= 0.0 {
            return None;
        }
        Some(self.output_tokens as f64 / self.service_latency_s)
    }

    /// Fill prefill/decode proxies from TTFT, connect, and token counts.
    pub fn with_phase_metrics(mut self) -> Self {
        self.prefill_s = match (self.ttft_s, self.connect_s) {
            (Some(ttft), Some(connect)) => Some((ttft - connect).max(0.0)),
            (Some(ttft), None) => Some(ttft),
            _ => None,
        };
        self.decode_s = self
            .ttft_s
            .map(|ttft| (self.service_latency_s - ttft).max(0.0));
        self.decode_tok_s = match self.decode_s {
            Some(decode) if decode > 0.0 && self.output_tokens > 0 => {
                Some(self.output_tokens as f64 / decode)
            }
            _ => None,
        };
        self
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SweepPoint {
    pub load: f64,
    pub n: usize,
    pub errors: usize,
    pub throughput: f64,
    /// Type-7 latency distribution over successful requests.
    pub latency_s: crate::stats::DistSummary,
    pub p50_s: Option<f64>,
    pub p95_s: Option<f64>,
    pub p99_s: Option<f64>,
    pub p99_unreliable: bool,
    pub error_rate: Option<f64>,
    pub validity_rate: Option<f64>,
    /// Schema-valid successes that also meet optional `--slo` thresholds.
    /// Without SLOs this equals validity-filtered throughput (often == throughput).
    pub goodput: f64,
    /// True when no SLO thresholds were applied (goodput is validity-only).
    pub goodput_equals_throughput: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slo_thresholds_s: Option<std::collections::BTreeMap<String, f64>>,
    /// Per-request output tok/s (`output_tokens / service_latency_s`) over successes.
    pub user_tps: crate::stats::DistSummary,
    /// Effective concurrent users meeting `user_tps=` at this stage load:
    /// `load * (meeting / successes)`. Null when `user_tps=` is unset.
    pub users_at_slo: Option<f64>,
    /// Successes that individually meet the `user_tps=` threshold (when set).
    pub users_meeting_user_tps: Option<usize>,
    /// Stage output-token throughput (success tokens / window).
    pub completion_tokens_per_second: Option<f64>,
    /// `$ / 1M output tokens` from declared price and stage token rate; null when absent.
    pub cost_per_million_output_tokens: Option<f64>,
    /// Client-observed outstanding concurrency vs stage cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_concurrency: Option<crate::concurrency::ObservedConcurrency>,
    pub connect_s: crate::stats::DistSummary,
    pub prefill_s: crate::stats::DistSummary,
    pub decode_s: crate::stats::DistSummary,
    pub decode_tok_s: crate::stats::DistSummary,
    /// Runtime ISL/OSL vs optional targets for this stage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isl_osl: Option<crate::isl_osl::IslOslValidation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
}

fn strategic_meets_slos(record: &BenchRecord, slos: &crate::summary::SloConfig) -> bool {
    if let Some(limit) = slos.e2e_s {
        if record.latency_s > limit {
            return false;
        }
    }
    // TTFT/TPOT apply only when the request carries those timings (streaming).
    if let (Some(limit), Some(ttft)) = (slos.ttft_s, record.ttft_s) {
        if ttft > limit {
            return false;
        }
    }
    if let (Some(limit), Some(tpot)) = (slos.tpot_s, record.tpot_s()) {
        if tpot > limit {
            return false;
        }
    }
    if let Some(min_rate) = slos.user_tps {
        match record.user_tps() {
            Some(rate) if rate >= min_rate => {}
            _ => return false,
        }
    }
    true
}

pub fn summarize_stage(
    load: f64,
    records: &[BenchRecord],
    seconds: f64,
    slos: &crate::summary::SloConfig,
    config: Option<Value>,
) -> SweepPoint {
    summarize_stage_with_options(load, records, seconds, slos, config, None, None, None)
}

/// Like [`summarize_stage`] but optionally stamps `$ / 1M output tokens`.
pub fn summarize_stage_with_price(
    load: f64,
    records: &[BenchRecord],
    seconds: f64,
    slos: &crate::summary::SloConfig,
    config: Option<Value>,
    price_per_hour: Option<f64>,
) -> SweepPoint {
    summarize_stage_with_options(
        load,
        records,
        seconds,
        slos,
        config,
        price_per_hour,
        None,
        None,
    )
}

/// Full stage summary with optional price, observed concurrency, and ISL/OSL.
#[allow(clippy::too_many_arguments)]
pub fn summarize_stage_with_options(
    load: f64,
    records: &[BenchRecord],
    seconds: f64,
    slos: &crate::summary::SloConfig,
    config: Option<Value>,
    price_per_hour: Option<f64>,
    observed_concurrency: Option<crate::concurrency::ObservedConcurrency>,
    isl_osl: Option<crate::isl_osl::IslOslValidation>,
) -> SweepPoint {
    // Warmup rows stay in the CSV for audit but never enter knee / HTML aggregates.
    let measured: Vec<&BenchRecord> = records.iter().filter(|record| !record.warmup).collect();
    let success_rows: Vec<&BenchRecord> = measured
        .iter()
        .copied()
        .filter(|record| record.success)
        .collect();
    let success_lats: Vec<f64> = success_rows.iter().map(|record| record.latency_s).collect();
    let latency_s = crate::stats::DistSummary::from_values(&success_lats);
    let successes = success_lats.len();
    let errors = measured.len().saturating_sub(successes);
    let valid = measured
        .iter()
        .filter(|record| record.success && record.valid.unwrap_or(true))
        .count();
    let validity_count = measured
        .iter()
        .filter(|record| record.valid.is_some())
        .count();
    let good = measured
        .iter()
        .filter(|record| {
            record.success && record.valid.unwrap_or(true) && strategic_meets_slos(record, slos)
        })
        .count();
    let elapsed = seconds.max(f64::EPSILON);
    let user_rates: Vec<f64> = success_rows
        .iter()
        .filter_map(|record| record.user_tps())
        .collect();
    let user_tps = crate::stats::DistSummary::from_values(&user_rates);
    let meeting_user_tps = slos.user_tps.map(|min_rate| {
        success_rows
            .iter()
            .filter(|record| record.user_tps().is_some_and(|rate| rate >= min_rate))
            .count()
    });
    let users_at_slo = meeting_user_tps.map(|meeting| {
        if successes == 0 {
            0.0
        } else {
            load * (meeting as f64 / successes as f64)
        }
    });
    let output_tokens: u64 = success_rows.iter().map(|record| record.output_tokens).sum();
    let completion_tokens_per_second = (successes > 0).then_some(output_tokens as f64 / elapsed);
    let cost_per_million_output_tokens = match (price_per_hour, completion_tokens_per_second) {
        (Some(price), Some(rate)) => crate::summary::cost_per_million_output_tokens(price, rate),
        _ => None,
    };
    let connect: Vec<f64> = success_rows
        .iter()
        .filter_map(|record| record.connect_s)
        .collect();
    let prefill: Vec<f64> = success_rows
        .iter()
        .filter_map(|record| record.prefill_s)
        .collect();
    let decode: Vec<f64> = success_rows
        .iter()
        .filter_map(|record| record.decode_s)
        .collect();
    let decode_tok: Vec<f64> = success_rows
        .iter()
        .filter_map(|record| record.decode_tok_s)
        .collect();
    let mut thresholds: std::collections::BTreeMap<String, f64> = [
        ("ttft", slos.ttft_s),
        ("tpot", slos.tpot_s),
        ("e2e", slos.e2e_s),
    ]
    .into_iter()
    .filter_map(|(name, value)| value.map(|v| (name.to_string(), v)))
    .collect();
    if let Some(rate) = slos.user_tps {
        thresholds.insert("user_tps".to_string(), rate);
    }
    let no_slos = thresholds.is_empty();
    SweepPoint {
        load,
        n: measured.len(),
        errors,
        throughput: successes as f64 / elapsed,
        latency_s: latency_s.clone(),
        p50_s: latency_s.p50,
        p95_s: latency_s.p95,
        p99_s: latency_s.p99,
        p99_unreliable: latency_s.p99_unreliable,
        error_rate: if measured.is_empty() {
            None
        } else {
            Some(errors as f64 / measured.len() as f64)
        },
        validity_rate: (validity_count > 0).then_some(valid as f64 / successes.max(1) as f64),
        goodput: good as f64 / elapsed,
        goodput_equals_throughput: no_slos,
        slo_thresholds_s: (!no_slos).then_some(thresholds),
        user_tps,
        users_at_slo,
        users_meeting_user_tps: meeting_user_tps,
        completion_tokens_per_second,
        cost_per_million_output_tokens,
        observed_concurrency,
        connect_s: crate::stats::DistSummary::from_values(&connect),
        prefill_s: crate::stats::DistSummary::from_values(&prefill),
        decode_s: crate::stats::DistSummary::from_values(&decode),
        decode_tok_s: crate::stats::DistSummary::from_values(&decode_tok),
        isl_osl,
        config,
    }
}

/// Finds the maximum distance from the endpoint chord after normalizing the
/// throughput/latency curve. This is the standard deterministic Kneedle
/// construction and is robust to units and uneven sweep spacing.
pub fn detect_knee(points: &[SweepPoint]) -> Option<usize> {
    if points.len() < 3 {
        return None;
    }
    let x_min = points.first()?.throughput;
    let x_max = points.last()?.throughput;
    let y_min = points.first()?.p95_s?;
    let y_max = points.last()?.p95_s?;
    if (x_max - x_min).abs() <= f64::EPSILON || (y_max - y_min).abs() <= f64::EPSILON {
        return None;
    }
    points
        .iter()
        .enumerate()
        .skip(1)
        .take(points.len() - 2)
        .filter_map(|(index, point)| {
            let x = (point.throughput - x_min) / (x_max - x_min);
            point
                .p95_s
                .map(|latency| (index, ((latency - y_min) / (y_max - y_min) - x).abs()))
        })
        .max_by(|left, right| left.1.total_cmp(&right.1))
        .map(|pair| pair.0)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerMetrics {
    pub kv_cache_usage: Option<f64>,
    pub preemptions: Option<f64>,
    pub requests_running: Option<f64>,
    pub requests_waiting: Option<f64>,
}

fn prometheus_values<'a>(text: &'a str, names: &'a [&'a str]) -> impl Iterator<Item = f64> + 'a {
    text.lines().filter_map(|line| {
        let line = line.trim();
        let name = line.split(['{', ' ', '\t']).next().unwrap_or_default();
        (!line.starts_with('#') && names.contains(&name))
            .then(|| line.split_whitespace().last()?.parse().ok())
            .flatten()
    })
}

pub fn parse_server_metrics(text: &str) -> ServerMetrics {
    ServerMetrics {
        kv_cache_usage: prometheus_values(
            text,
            &[
                "vllm:gpu_cache_usage_perc",
                "vllm:kv_cache_usage_perc",
                "sglang:cache_hit_rate",
                "trtllm_kv_cache_utilization",
            ],
        )
        .max_by(f64::total_cmp),
        preemptions: prometheus_values(
            text,
            &[
                "vllm:num_preemptions_total",
                "sglang:num_preemptions_total",
                "trtllm_request_preemptions",
            ],
        )
        .reduce(|sum, value| sum + value),
        requests_running: prometheus_values(
            text,
            &[
                "vllm:num_requests_running",
                "sglang:num_running_reqs",
                "trtllm_request_metrics_active",
            ],
        )
        .reduce(|sum, value| sum + value),
        requests_waiting: prometheus_values(
            text,
            &[
                "vllm:num_requests_waiting",
                "sglang:num_queue_reqs",
                "trtllm_request_metrics_queued",
            ],
        )
        .reduce(|sum, value| sum + value),
    }
}

pub async fn scrape_metrics(client: &reqwest::Client, url: &str) -> Result<ServerMetrics> {
    let text = client
        .get(url)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .with_context(|| format!("scrape {url}"))?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_server_metrics(&text))
}

#[derive(Debug, Clone, Deserialize)]
pub struct Session {
    pub session_id: String,
    pub messages: Vec<Value>,
}

pub fn load_sessions(path: &Path) -> Result<Vec<Session>> {
    let reader = BufReader::new(File::open(path)?);
    let mut sessions = Vec::new();
    for (line_number, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let session: Session = serde_json::from_str(&line)
            .with_context(|| format!("invalid session JSONL line {}", line_number + 1))?;
        if session.messages.is_empty() {
            bail!("session {} has no messages", session.session_id);
        }
        sessions.push(session);
    }
    if sessions.is_empty() {
        bail!("session dataset is empty");
    }
    Ok(sessions)
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum PrefixControl {
    Shared,
    Unique,
    None,
}

pub fn controlled_messages(
    messages: &[Value],
    shared_prefix: Option<&str>,
    control: PrefixControl,
    session_id: &str,
) -> Vec<Value> {
    let mut output = Vec::new();
    if let Some(prefix) = shared_prefix {
        if !matches!(control, PrefixControl::None) {
            let content = match control {
                PrefixControl::Shared => prefix.to_string(),
                PrefixControl::Unique => format!("[session:{session_id}] {prefix}"),
                PrefixControl::None => unreachable!(),
            };
            output.push(json!({"role": "system", "content": content}));
        }
    }
    output.extend_from_slice(messages);
    output
}

#[derive(Debug, Clone)]
pub enum Validity {
    Json {
        validator: Arc<jsonschema::Validator>,
    },
    ToolCall {
        validators: BTreeMap<String, Arc<jsonschema::Validator>>,
    },
}

impl Validity {
    pub fn json_schema(schema: &Value) -> Result<Self> {
        let validator = jsonschema::validator_for(schema)
            .map_err(|error| anyhow::anyhow!("invalid JSON schema: {error}"))?;
        Ok(Self::Json {
            validator: Arc::new(validator),
        })
    }

    pub fn tool_names(tools: &Value) -> Result<Self> {
        let array = tools.as_array().context("tools must be a JSON array")?;
        if array.is_empty() {
            bail!("tools array must not be empty");
        }
        let mut validators = BTreeMap::new();
        for tool in array {
            let name = tool
                .pointer("/function/name")
                .and_then(Value::as_str)
                .context("each tool must have function.name")?;
            let parameters = tool
                .pointer("/function/parameters")
                .cloned()
                .unwrap_or_else(|| json!({"type":"object"}));
            let validator = jsonschema::validator_for(&parameters)
                .map_err(|error| anyhow::anyhow!("invalid schema for tool {name}: {error}"))?;
            if validators
                .insert(name.to_string(), Arc::new(validator))
                .is_some()
            {
                bail!("duplicate tool name: {name}");
            }
        }
        Ok(Self::ToolCall { validators })
    }

    pub fn validate(&self, response: &Value) -> bool {
        match self {
            Self::Json { validator } => {
                let content = response
                    .pointer("/choices/0/message/content")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                serde_json::from_str::<Value>(content)
                    .ok()
                    .is_some_and(|value| validator.is_valid(&value))
            }
            Self::ToolCall { validators } => response
                .pointer("/choices/0/message/tool_calls")
                .and_then(Value::as_array)
                .is_some_and(|calls| {
                    !calls.is_empty()
                        && calls.iter().all(|call| {
                            let name = call.pointer("/function/name").and_then(Value::as_str);
                            let arguments =
                                call.pointer("/function/arguments").and_then(Value::as_str);
                            name.and_then(|name| validators.get(name))
                                .zip(
                                    arguments
                                        .and_then(|text| serde_json::from_str::<Value>(text).ok()),
                                )
                                .is_some_and(|(validator, arguments)| {
                                    validator.is_valid(&arguments)
                                })
                        })
                }),
        }
    }
}

pub fn export_csv(path: &Path, records: &[BenchRecord]) -> Result<()> {
    let mut writer = csv::Writer::from_path(path)?;
    for record in records {
        writer.serialize(record)?;
    }
    writer.flush()?;
    Ok(())
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn nice_tick(span: f64) -> f64 {
    if span <= 0.0 || !span.is_finite() {
        return 1.0;
    }
    let exp = span.log10().floor();
    let base = 10f64.powf(exp);
    let frac = span / base;
    let nice = if frac <= 1.5 {
        1.0
    } else if frac <= 3.0 {
        2.0
    } else if frac <= 7.0 {
        5.0
    } else {
        10.0
    };
    nice * base / 4.0
}

fn format_tick(value: f64) -> String {
    if value == 0.0 {
        "0".into()
    } else if value.abs() >= 100.0 {
        format!("{value:.0}")
    } else if value.abs() >= 10.0 {
        format!("{value:.1}")
    } else if value.abs() >= 1.0 {
        format!("{value:.2}")
    } else {
        format!("{value:.3}")
    }
}

/// Emit a static HTML report with labeled axes, ticks, and a sweep table.
pub fn export_html(
    path: &Path,
    title: &str,
    points: &[SweepPoint],
    knee: Option<usize>,
    server: &ServerMetrics,
    sut: Option<&Value>,
) -> Result<()> {
    let width = 760.0;
    let height = 340.0;
    let margin_left = 64.0;
    let margin_right = 24.0;
    let margin_top = 24.0;
    let margin_bottom = 52.0;
    let plot_w = width - margin_left - margin_right;
    let plot_h = height - margin_top - margin_bottom;

    let max_x = points
        .iter()
        .map(|point| point.throughput)
        .fold(0.0_f64, f64::max)
        .max(1.0);
    let max_y = points
        .iter()
        .filter_map(|point| point.p95_s)
        .fold(0.0_f64, f64::max)
        .max(0.001);

    let map_x = |throughput: f64| margin_left + throughput / max_x * plot_w;
    let map_y = |p95: f64| margin_top + plot_h - p95 / max_y * plot_h;

    let coordinates: Vec<_> = points
        .iter()
        .map(|point| {
            (
                map_x(point.throughput),
                map_y(point.p95_s.unwrap_or_default()),
            )
        })
        .collect();
    let polyline = coordinates
        .iter()
        .map(|(x, y)| format!("{x:.1},{y:.1}"))
        .collect::<Vec<_>>()
        .join(" ");

    let mut grid = String::new();
    let x_step = nice_tick(max_x).max(max_x / 5.0);
    let mut x_tick = 0.0;
    while x_tick <= max_x + x_step * 0.01 {
        let x = map_x(x_tick);
        grid.push_str(&format!(
            r##"<line x1="{x:.1}" y1="{margin_top}" x2="{x:.1}" y2="{y2:.1}" stroke="#e5e7eb" stroke-width="1"/>"##,
            y2 = margin_top + plot_h
        ));
        grid.push_str(&format!(
            r##"<text x="{x:.1}" y="{y:.1}" text-anchor="middle" font-size="11" fill="#374151">{label}</text>"##,
            y = margin_top + plot_h + 16.0,
            label = escape_html(&format_tick(x_tick))
        ));
        x_tick += x_step;
        if x_step <= f64::EPSILON {
            break;
        }
    }
    let y_step = nice_tick(max_y).max(max_y / 5.0);
    let mut y_tick = 0.0;
    while y_tick <= max_y + y_step * 0.01 {
        let y = map_y(y_tick);
        grid.push_str(&format!(
            r##"<line x1="{margin_left}" y1="{y:.1}" x2="{x2:.1}" y2="{y:.1}" stroke="#e5e7eb" stroke-width="1"/>"##,
            x2 = margin_left + plot_w
        ));
        grid.push_str(&format!(
            r##"<text x="{x:.1}" y="{y:.1}" text-anchor="end" dominant-baseline="middle" font-size="11" fill="#374151">{label}</text>"##,
            x = margin_left - 8.0,
            label = escape_html(&format_tick(y_tick))
        ));
        y_tick += y_step;
        if y_step <= f64::EPSILON {
            break;
        }
    }

    let axis = format!(
        r##"<line x1="{margin_left}" y1="{y0:.1}" x2="{x1:.1}" y2="{y0:.1}" stroke="#111827" stroke-width="1.5"/>
<line x1="{margin_left}" y1="{margin_top}" x2="{margin_left}" y2="{y0:.1}" stroke="#111827" stroke-width="1.5"/>
<text x="{cx:.1}" y="{xlabel_y:.1}" text-anchor="middle" font-size="13" font-weight="600" fill="#111827">Throughput (req/s)</text>
<text x="16" y="{cy:.1}" text-anchor="middle" font-size="13" font-weight="600" fill="#111827" transform="rotate(-90 16 {cy:.1})">p95 latency (s)</text>"##,
        y0 = margin_top + plot_h,
        x1 = margin_left + plot_w,
        cx = margin_left + plot_w / 2.0,
        xlabel_y = height - 12.0,
        cy = margin_top + plot_h / 2.0,
    );

    let dots: String = coordinates
        .iter()
        .enumerate()
        .map(|(index, (x, y))| {
            let label = points
                .get(index)
                .map(|p| {
                    format!(
                        "load={:.3} thr={:.2} p95={}",
                        p.load,
                        p.throughput,
                        format_tick(p.p95_s.unwrap_or_default())
                    )
                })
                .unwrap_or_default();
            format!(
                r##"<circle cx="{x:.1}" cy="{y:.1}" r="3.5" fill="#2563eb"><title>{}</title></circle>"##,
                escape_html(&label)
            )
        })
        .collect();

    let rows = points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            format!(
                "<tr{}><td>{:.2}</td><td>{}</td><td>{:.2}</td><td>{}</td><td>{}</td><td>{}</td><td>{:.2}</td></tr>",
                if knee == Some(index) {
                    " class=knee"
                } else {
                    ""
                },
                point.load,
                point.n,
                point.throughput,
                point
                    .p95_s
                    .map(|value| format!("{value:.3}"))
                    .unwrap_or_else(|| "-".to_string()),
                point
                    .p99_s
                    .map(|value| format!("{value:.3}"))
                    .unwrap_or_else(|| "-".to_string()),
                point
                    .error_rate
                    .map(|value| format!("{:.2}%", value * 100.0))
                    .unwrap_or_else(|| "-".to_string()),
                point.goodput
            )
        })
        .collect::<String>();
    let knee_circle = knee
        .and_then(|index| coordinates.get(index))
        .map(|(x, y)| {
            format!(
                r##"<circle cx="{x:.1}" cy="{y:.1}" r="7" fill="#ef4444"><title>knee</title></circle>"##
            )
        })
        .unwrap_or_default();
    let sut_block = match sut {
        Some(value) => format!(
            "<h2>System under test</h2><pre>{}</pre>",
            escape_html(&serde_json::to_string_pretty(value)?)
        ),
        None => String::new(),
    };
    let html = format!(
        r##"<!doctype html><html><head><meta charset="utf-8"><title>{title}</title>
<style>body{{font:14px system-ui,sans-serif;margin:2rem;max-width:960px;color:#111827}}table{{border-collapse:collapse;width:100%}}th,td{{padding:.45rem;border-bottom:1px solid #ddd;text-align:right}}th:first-child,td:first-child{{text-align:left}}.knee{{background:#fee2e2}}svg{{border:1px solid #d1d5db;background:#fafafa;max-width:100%}}caption{{text-align:left;font-weight:600;margin:.5rem 0}}</style></head>
<body><h1>{title}</h1>
<p>Latency-throughput curve with labeled axes. Red marks the automatically detected knee (when present).</p>
<svg viewBox="0 0 {width} {height}" role="img" aria-label="p95 latency versus throughput with labeled axes">
{grid}{axis}
<polyline points="{polyline}" fill="none" stroke="#2563eb" stroke-width="3"/>
{dots}{knee_circle}
</svg>
<h2>Sweep</h2>
<table><caption>Per-stage load, throughput, latency, and goodput</caption><thead><tr><th>Load</th><th>n</th><th>Throughput (req/s)</th><th>p95 seconds</th><th>p99 seconds</th><th>Error</th><th>Goodput</th></tr></thead><tbody>{rows}</tbody></table>
{sut_block}
<h2>Server correlation</h2><pre>{server}</pre></body></html>"##,
        title = escape_html(title),
        server = escape_html(&serde_json::to_string_pretty(server)?),
    );
    std::fs::write(path, html)?;
    Ok(())
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum MlperfScenario {
    Server,
    Offline,
}

pub fn export_mlperf(
    directory: &Path,
    scenario: MlperfScenario,
    records: &[BenchRecord],
    duration_s: f64,
) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    let completed = records.iter().filter(|record| record.success).count();
    let qps = completed as f64 / duration_s.max(f64::EPSILON);
    let latencies = crate::stats::sort_finite(
        records
            .iter()
            .filter(|record| record.success)
            .map(|record| record.latency_s),
    );
    let scenario_name = match scenario {
        MlperfScenario::Server => "Server",
        MlperfScenario::Offline => "Offline",
    };
    const DISCLAIMER: &str = "UNOFFICIAL: This is NOT an audited or submitted MLPerf result. Parser-oriented interoperability export only; do not treat as an official MLPerf LoadGen run.";
    let mut summary = File::create(directory.join("mlperf_log_summary.txt"))?;
    writeln!(summary, "{DISCLAIMER}")?;
    writeln!(summary, "MLPerf Results Summary")?;
    writeln!(summary, "SUT name : Metrum AI Bench")?;
    writeln!(summary, "Scenario : {scenario_name}")?;
    writeln!(summary, "Mode : PerformanceOnly")?;
    match scenario {
        MlperfScenario::Server => {
            writeln!(summary, "Scheduled samples per second : {qps:.6}")?;
            writeln!(summary, "Completed samples per second : {qps:.6}")?;
            for percentile in [50.0, 90.0, 95.0, 97.0, 99.0, 99.9] {
                if let Some(latency) = crate::stats::percentile_type7(&latencies, percentile) {
                    writeln!(
                        summary,
                        "{percentile:.2} percentile latency (ns) : {:.0}",
                        latency * 1e9
                    )?;
                }
            }
        }
        MlperfScenario::Offline => {
            writeln!(summary, "Samples per second : {qps:.6}")?;
        }
    }
    writeln!(summary, "Test Parameters Used")?;
    writeln!(summary, "samples_per_query : {}", records.len())?;
    writeln!(summary, "duration (s) : {duration_s:.6}")?;
    writeln!(
        summary,
        "Result validity : {}",
        if !records.is_empty() && completed == records.len() {
            "UNOFFICIAL_OK (see disclaimer; not an official MLPerf VALID)"
        } else {
            "UNOFFICIAL_INVALID (see disclaimer)"
        }
    )?;
    let mut detail = File::create(directory.join("mlperf_log_detail.txt"))?;
    writeln!(detail, "{DISCLAIMER}")?;
    for record in records {
        writeln!(
            detail,
            ":::MLLOG {{\"key\":\"sample\",\"value\":{{\"id\":{},\"scheduled_time_ns\":{},\"sent_time_ns\":{},\"latency_ns\":{:.0},\"success\":{}}},\"metadata\":{{\"file\":\"metrum-ai-bench-cli\",\"lineno\":0}}}}",
            record.seq,
            record.scheduled_unix_ns,
            record.sent_unix_ns,
            record.latency_s * 1e9,
            record.success
        )?;
    }
    let accuracy = serde_json::json!({
        "disclaimer": DISCLAIMER,
        "accuracy": []
    });
    File::create(directory.join("mlperf_log_accuracy.json"))?
        .write_all(format!("{accuracy}\n").as_bytes())?;
    Ok(())
}

/// Sends OTLP/HTTP JSON mappings. Compiled only when explicitly enabled so
/// normal installations have no telemetry behavior.
#[cfg(feature = "otlp")]
pub async fn export_otlp(
    client: &reqwest::Client,
    endpoint: &str,
    service_name: &str,
    records: &[BenchRecord],
    headers: &BTreeMap<String, String>,
) -> Result<()> {
    let spans: Vec<_> = records
        .iter()
        .map(|record| {
            let trace_id = format!("{:032x}", record.scheduled_unix_ns ^ record.seq as u128);
            let span_id = format!("{:016x}", (record.sent_unix_ns as u64) ^ record.seq);
            json!({
                "traceId": trace_id, "spanId": span_id, "name": "inference.request",
                "kind": 3, "startTimeUnixNano": record.scheduled_unix_ns.to_string(),
                "endTimeUnixNano": (record.scheduled_unix_ns + (record.latency_s * 1e9) as u128).to_string(),
                "attributes": [
                    {"key":"benchmark.stage","value":{"doubleValue":record.stage}},
                    {"key":"benchmark.success","value":{"boolValue":record.success}},
                    {"key":"benchmark.queue_delay_s","value":{"doubleValue":record.queue_delay_s}},
                    {"key":"benchmark.service_latency_s","value":{"doubleValue":record.service_latency_s}},
                    {"key":"server.address","value":{"stringValue":record.endpoint}}
                ],
                "status":{"code": if record.success { 1 } else { 2 }}
            })
        })
        .collect();
    let body = json!({"resourceSpans":[{
        "resource":{"attributes":[{"key":"service.name","value":{"stringValue":service_name}}]},
        "scopeSpans":[{"scope":{"name":"metrum-ai-bench-cli"},"spans":spans}]
    }]});
    let url = format!("{}/v1/traces", endpoint.trim_end_matches('/'));
    let mut request = client.post(url).json(&body);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    request.send().await?.error_for_status()?;
    let now = now_unix_ns().to_string();
    let successes = records.iter().filter(|record| record.success).count();
    let latency_values = crate::stats::sort_finite(
        records
            .iter()
            .filter(|record| record.success)
            .map(|record| record.latency_s),
    );
    let mut metrics = vec![
        json!({"name":"benchmark.requests","unit":"{request}","sum":{
            "aggregationTemporality":2,"isMonotonic":true,
            "dataPoints":[{"asInt":records.len().to_string(),"timeUnixNano":now}]
        }}),
        json!({"name":"benchmark.successes","unit":"{request}","sum":{
            "aggregationTemporality":2,"isMonotonic":true,
            "dataPoints":[{"asInt":successes.to_string(),"timeUnixNano":now}]
        }}),
    ];
    if let Some(p95) = crate::stats::percentile_type7(&latency_values, 95.0) {
        metrics.push(json!({"name":"benchmark.latency.p95","unit":"s","gauge":{
            "dataPoints":[{"asDouble":p95,"timeUnixNano":now}]
        }}));
    }
    let metric_body = json!({"resourceMetrics":[{
        "resource":{"attributes":[{"key":"service.name","value":{"stringValue":service_name}}]},
        "scopeMetrics":[{"scope":{"name":"metrum-ai-bench-cli"},"metrics":metrics}]
    }]});
    let metrics_url = format!("{}/v1/metrics", endpoint.trim_end_matches('/'));
    let mut request = client.post(metrics_url).json(&metric_body);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    request.send().await?.error_for_status()?;
    Ok(())
}

pub fn now_unix_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knee_finds_curve_bend() {
        let points: Vec<_> = [(1.0, 1.0), (2.0, 1.1), (3.0, 1.3), (3.2, 4.0)]
            .into_iter()
            .map(|(throughput, latency)| {
                let latency_s = crate::stats::DistSummary::from_values(&[latency]);
                SweepPoint {
                    load: throughput,
                    n: 1,
                    errors: 0,
                    throughput,
                    latency_s: latency_s.clone(),
                    p50_s: Some(latency),
                    p95_s: Some(latency),
                    p99_s: Some(latency),
                    p99_unreliable: latency_s.p99_unreliable,
                    error_rate: Some(0.0),
                    validity_rate: None,
                    goodput: throughput,
                    goodput_equals_throughput: true,
                    slo_thresholds_s: None,
                    user_tps: crate::stats::DistSummary::from_values(&[]),
                    users_at_slo: None,
                    users_meeting_user_tps: None,
                    completion_tokens_per_second: None,
                    cost_per_million_output_tokens: None,
                    observed_concurrency: None,
                    connect_s: crate::stats::DistSummary::from_values(&[]),
                    prefill_s: crate::stats::DistSummary::from_values(&[]),
                    decode_s: crate::stats::DistSummary::from_values(&[]),
                    decode_tok_s: crate::stats::DistSummary::from_values(&[]),
                    isl_osl: None,
                    config: None,
                }
            })
            .collect();
        assert_eq!(detect_knee(&points), Some(2));
    }

    #[test]
    fn html_report_has_axes_ticks_and_labels() {
        let latency_s = crate::stats::DistSummary::from_values(&[0.1, 0.2]);
        let points = vec![SweepPoint {
            load: 1.0,
            n: 2,
            errors: 0,
            throughput: 10.0,
            latency_s: latency_s.clone(),
            p50_s: Some(0.1),
            p95_s: Some(0.2),
            p99_s: Some(0.2),
            p99_unreliable: true,
            error_rate: Some(0.0),
            validity_rate: None,
            goodput: 10.0,
            goodput_equals_throughput: true,
            slo_thresholds_s: None,
            user_tps: crate::stats::DistSummary::from_values(&[]),
            users_at_slo: None,
            users_meeting_user_tps: None,
            completion_tokens_per_second: None,
            cost_per_million_output_tokens: None,
            observed_concurrency: None,
            connect_s: crate::stats::DistSummary::from_values(&[]),
            prefill_s: crate::stats::DistSummary::from_values(&[]),
            decode_s: crate::stats::DistSummary::from_values(&[]),
            decode_tok_s: crate::stats::DistSummary::from_values(&[]),
            isl_osl: None,
            config: None,
        }];
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.html");
        export_html(
            &path,
            "unit test",
            &points,
            None,
            &ServerMetrics::default(),
            None,
        )
        .unwrap();
        let html = std::fs::read_to_string(&path).unwrap();
        assert!(html.contains("Throughput (req/s)"));
        assert!(html.contains("p95 latency (s)"));
        assert!(html.contains("text-anchor"));
        assert!(html.contains("<polyline"));
    }

    #[test]
    fn parses_vllm_metrics() {
        let metrics = parse_server_metrics(
            "vllm:gpu_cache_usage_perc 0.75\nvllm:num_requests_waiting 3\nvllm:num_preemptions_total 2\n",
        );
        assert_eq!(metrics.kv_cache_usage, Some(0.75));
        assert_eq!(metrics.requests_waiting, Some(3.0));
    }

    #[test]
    fn validates_json_required_properties() {
        let validator = Validity::json_schema(&json!({
            "type":"object",
            "properties":{"answer":{"type":"integer"}},
            "required":["answer"],
            "additionalProperties":false
        }))
        .expect("valid schema");
        assert!(validator.validate(&json!({
            "choices":[{"message":{"content":"{\"answer\":42}"}}]
        })));
        assert!(!validator.validate(&json!({
            "choices":[{"message":{"content":"{}"}}]
        })));
        assert!(!validator.validate(&json!({
            "choices":[{"message":{"content":"{\"answer\":\"wrong type\"}"}}]
        })));
    }

    #[test]
    fn validates_tool_name_and_argument_schema() {
        let validator = Validity::tool_names(&json!([{
            "type":"function",
            "function":{
                "name":"lookup",
                "parameters":{
                    "type":"object",
                    "properties":{"id":{"type":"integer"}},
                    "required":["id"]
                }
            }
        }]))
        .expect("valid tools");
        let response = |name, arguments| {
            json!({"choices":[{"message":{"tool_calls":[{
                "function":{"name":name,"arguments":arguments}
            }]}}]})
        };
        assert!(validator.validate(&response("lookup", r#"{"id":1}"#)));
        assert!(!validator.validate(&response("lookup", r#"{"id":"1"}"#)));
        assert!(!validator.validate(&response("other", r#"{"id":1}"#)));
    }

    #[test]
    fn unique_prefix_is_session_scoped() {
        let messages = vec![json!({"role":"user","content":"hello"})];
        let controlled =
            controlled_messages(&messages, Some("common"), PrefixControl::Unique, "s1");
        assert_eq!(controlled[0]["content"], "[session:s1] common");
    }

    #[test]
    fn summarize_stage_excludes_warmup_records() {
        let measured = BenchRecord {
            seq: 1,
            stage: 1.0,
            endpoint: "http://example.test".to_string(),
            scheduled_unix_ns: 1,
            sent_unix_ns: 1,
            latency_s: 0.5,
            queue_delay_s: 0.0,
            service_latency_s: 0.5,
            first_byte_s: None,
            connect_s: None,
            ttft_s: None,
            prefill_s: None,
            decode_s: None,
            decode_tok_s: None,
            itl_s: Vec::new(),
            in_flight_at_send: None,
            success: true,
            valid: None,
            input_tokens: 1,
            output_tokens: 1,
            session_id: None,
            turn: None,
            error: None,
            warmup: false,
        };
        let mut cold = measured.clone();
        cold.seq = 0;
        cold.latency_s = 50.0;
        cold.service_latency_s = 50.0;
        cold.warmup = true;
        let point = summarize_stage(
            1.0,
            &[cold, measured],
            1.0,
            &crate::summary::SloConfig::default(),
            None,
        );
        assert_eq!(point.n, 1);
        assert_eq!(point.p50_s, Some(0.5));
        assert!(point.p95_s.unwrap() < 1.0);
    }

    #[test]
    fn sweep_uses_type7_and_null_for_undefined_latency() {
        let record = |seq, latency_s| BenchRecord {
            seq,
            stage: 1.0,
            endpoint: "http://example.test".to_string(),
            scheduled_unix_ns: 1,
            sent_unix_ns: 1,
            latency_s,
            queue_delay_s: 0.0,
            service_latency_s: latency_s,
            first_byte_s: None,
            connect_s: None,
            ttft_s: None,
            prefill_s: None,
            decode_s: None,
            decode_tok_s: None,
            itl_s: Vec::new(),
            in_flight_at_send: None,
            success: true,
            valid: None,
            input_tokens: 0,
            output_tokens: 0,
            session_id: None,
            turn: None,
            error: None,
            warmup: false,
        };
        let slos = crate::summary::SloConfig {
            ttft_s: Some(0.5),
            ..Default::default()
        };
        let mut timed = record(0, 1.0);
        assert!(strategic_meets_slos(&timed, &slos));
        timed.ttft_s = Some(0.5);
        assert!(strategic_meets_slos(&timed, &slos));
        timed.ttft_s = Some(0.6);
        assert!(!strategic_meets_slos(&timed, &slos));
        let point = summarize_stage(1.0, &[timed], 1.0, &slos, None);
        assert_eq!(point.goodput, 0.0);
        assert_eq!(point.throughput, 1.0);

        let point = summarize_stage(
            1.0,
            &[
                record(0, 1.0),
                record(1, 2.0),
                record(2, 3.0),
                record(3, 4.0),
            ],
            1.0,
            &crate::summary::SloConfig::default(),
            None,
        );
        assert_eq!(point.n, 4);
        assert_eq!(point.errors, 0);
        assert!(point.goodput_equals_throughput);
        assert!((point.p95_s.expect("p95") - 3.85).abs() < 1e-12);

        let empty = summarize_stage(1.0, &[], 1.0, &crate::summary::SloConfig::default(), None);
        let value = serde_json::to_value(empty).expect("serialize sweep point");
        assert!(value["p50_s"].is_null());
        assert!(value["p95_s"].is_null());
        assert!(value["p99_s"].is_null());
        assert!(value["error_rate"].is_null());
        assert!(value["cost_per_million_output_tokens"].is_null());
        assert!(value["users_at_slo"].is_null());
    }

    #[test]
    fn strategic_tpot_and_user_tps_slos() {
        let record = BenchRecord {
            seq: 0,
            stage: 1.0,
            endpoint: "http://example.test".to_string(),
            scheduled_unix_ns: 1,
            sent_unix_ns: 1,
            latency_s: 1.0,
            queue_delay_s: 0.0,
            service_latency_s: 1.0,
            first_byte_s: None,
            connect_s: None,
            ttft_s: Some(0.2),
            prefill_s: None,
            decode_s: None,
            decode_tok_s: None,
            itl_s: vec![0.05, 0.05],
            in_flight_at_send: None,
            success: true,
            valid: None,
            input_tokens: 0,
            output_tokens: 21,
            session_id: None,
            turn: None,
            error: None,
            warmup: false,
        };
        assert!((record.tpot_s().unwrap() - 0.04).abs() < 1e-12);
        assert!((record.user_tps().unwrap() - 21.0).abs() < 1e-12);

        let tpot_ok = crate::summary::SloConfig {
            tpot_s: Some(0.05),
            ..Default::default()
        };
        assert!(strategic_meets_slos(&record, &tpot_ok));
        let tpot_fail = crate::summary::SloConfig {
            tpot_s: Some(0.03),
            ..Default::default()
        };
        assert!(!strategic_meets_slos(&record, &tpot_fail));

        let user_ok = crate::summary::SloConfig {
            user_tps: Some(20.0),
            ..Default::default()
        };
        assert!(strategic_meets_slos(&record, &user_ok));
        let user_fail = crate::summary::SloConfig {
            user_tps: Some(25.0),
            ..Default::default()
        };
        assert!(!strategic_meets_slos(&record, &user_fail));

        let point = summarize_stage_with_price(4.0, &[record], 1.0, &user_ok, None, Some(3.6));
        assert_eq!(point.users_meeting_user_tps, Some(1));
        assert!((point.users_at_slo.unwrap() - 4.0).abs() < 1e-12);
        assert!((point.completion_tokens_per_second.unwrap() - 21.0).abs() < 1e-12);
        let expected = 3.6 / (21.0 * 3600.0) * 1_000_000.0;
        assert!((point.cost_per_million_output_tokens.unwrap() - expected).abs() < 1e-9);
    }

    #[test]
    fn mlperf_export_uses_scenario_fields_and_scheduled_latency() {
        let directory = tempfile::tempdir().expect("temporary MLPerf directory");
        let record = BenchRecord {
            seq: 7,
            stage: 10.0,
            endpoint: "http://example.test".to_string(),
            scheduled_unix_ns: 100,
            sent_unix_ns: 120,
            latency_s: 0.25,
            queue_delay_s: 0.02,
            service_latency_s: 0.23,
            first_byte_s: None,
            connect_s: None,
            ttft_s: None,
            prefill_s: None,
            decode_s: None,
            decode_tok_s: None,
            itl_s: Vec::new(),
            in_flight_at_send: None,
            success: true,
            valid: None,
            input_tokens: 1,
            output_tokens: 1,
            session_id: None,
            turn: None,
            error: None,
            warmup: false,
        };
        export_mlperf(directory.path(), MlperfScenario::Server, &[record], 1.0)
            .expect("export MLPerf logs");
        let summary = std::fs::read_to_string(directory.path().join("mlperf_log_summary.txt"))
            .expect("summary log");
        assert!(summary.contains("Scenario : Server"));
        assert!(summary.contains("90.00 percentile latency (ns) : 250000000"));
        assert!(summary.contains("UNOFFICIAL"));
        assert!(summary.contains("Result validity : UNOFFICIAL_OK"));
        assert!(
            !summary.contains("Result is : VALID"),
            "must not emit the official LoadGen VALID substring"
        );
        let detail = std::fs::read_to_string(directory.path().join("mlperf_log_detail.txt"))
            .expect("detail log");
        assert!(detail.starts_with("UNOFFICIAL"));
        assert!(detail.contains("\"scheduled_time_ns\":100"));
        assert!(detail.contains("\"sent_time_ns\":120"));
    }

    #[cfg(feature = "otlp")]
    #[tokio::test]
    async fn otlp_exports_traces_and_metrics() {
        use axum::extract::{OriginalUri, State};
        use axum::routing::post;
        use axum::{Json, Router};
        use std::sync::Arc;
        use tokio::sync::Mutex;

        type CapturedCalls = Arc<Mutex<Vec<(String, Value)>>>;

        async fn capture(
            State(calls): State<CapturedCalls>,
            OriginalUri(uri): OriginalUri,
            Json(body): Json<Value>,
        ) {
            calls.lock().await.push((uri.path().to_string(), body));
        }

        let calls = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route("/v1/traces", post(capture))
            .route("/v1/metrics", post(capture))
            .with_state(calls.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind collector");
        let address = listener.local_addr().expect("collector address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve collector");
        });
        let record = BenchRecord {
            seq: 1,
            stage: 5.0,
            endpoint: "http://example.test".to_string(),
            scheduled_unix_ns: 100,
            sent_unix_ns: 110,
            latency_s: 0.5,
            queue_delay_s: 0.1,
            service_latency_s: 0.4,
            first_byte_s: None,
            connect_s: None,
            ttft_s: None,
            prefill_s: None,
            decode_s: None,
            decode_tok_s: None,
            itl_s: Vec::new(),
            in_flight_at_send: None,
            success: true,
            valid: Some(true),
            input_tokens: 2,
            output_tokens: 3,
            session_id: None,
            turn: None,
            error: None,
            warmup: false,
        };
        export_otlp(
            &reqwest::Client::new(),
            &format!("http://{address}"),
            "test",
            &[record],
            &BTreeMap::new(),
        )
        .await
        .expect("OTLP export");
        server.abort();
        let calls = calls.lock().await;
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "/v1/traces");
        assert_eq!(calls[1].0, "/v1/metrics");
        assert_eq!(
            calls[0]
                .1
                .pointer("/resourceSpans/0/scopeSpans/0/spans/0/startTimeUnixNano"),
            Some(&json!("100"))
        );
        assert_eq!(
            calls[1]
                .1
                .pointer("/resourceMetrics/0/scopeMetrics/0/metrics/0/name"),
            Some(&json!("benchmark.requests"))
        );
    }
}
