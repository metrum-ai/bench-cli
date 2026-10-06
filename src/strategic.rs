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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    /// Provenance for `ttft_s`: `stream` or `first_byte_approx`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttft_source: Option<crate::measurement::TtftSource>,
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
    /// Send-to-first-reasoning-chunk timing for streaming thinking models.
    /// Last column so older CSV readers keep their positions.
    #[serde(default)]
    pub first_reasoning_s: Option<f64>,
    /// Server-reported reasoning tokens (chat only); empty when not reported.
    /// Appended after `first_reasoning_s` so older CSV readers keep positions (#192).
    #[serde(default)]
    pub reasoning_tokens: Option<u64>,
    /// HTTP phase trace (#194), appended after `reasoning_tokens` so older CSV
    /// readers keep positions. True when a pooled connection was reused.
    #[serde(default)]
    pub connection_reused: Option<bool>,
    /// DNS seconds inside the connector; `0.0` when no lookup ran.
    #[serde(default)]
    pub dns_s: Option<f64>,
    /// Request body bytes; empty when the length is unknown.
    #[serde(default)]
    pub bytes_sent: Option<u64>,
    /// Response headers to last body chunk; successes only.
    #[serde(default)]
    pub receive_s: Option<f64>,
    /// Response body bytes after content decoding; successes only.
    #[serde(default)]
    pub bytes_received: Option<u64>,
    /// Response body chunks yielded by the HTTP client; successes only.
    #[serde(default)]
    pub chunks_received: Option<u64>,
    /// Monotonic send time in seconds from the run start (the telemetry
    /// `t_sent_ns` origin). Stage windows use it, so a wall-clock (NTP) step
    /// cannot stretch them; empty in CSVs written before #224. Last column so
    /// older CSV readers keep their positions.
    #[serde(default)]
    pub send_offset_s: Option<f64>,
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
    /// Sum of server `usage` input tokens over stage successes that report
    /// usage; null when no success reports usage.
    pub prompt_tokens_total: Option<u64>,
    /// Sum of server `usage` output tokens over the same rows; null when no
    /// success reports usage or the stage generates no output (embeddings, rerank).
    pub completion_tokens_total: Option<u64>,
    /// `prompt_tokens_total / window`; null when the total is null.
    pub input_tokens_per_second: Option<f64>,
    /// `(prompt_tokens_total + completion_tokens_total) / window`; null unless both totals exist.
    pub total_tokens_per_second: Option<f64>,
    /// Per-request prefill rate `input_tokens / min(first_reasoning_s, ttft_s)` over successes with both
    /// and visible-token TTFT (first-byte approximations excluded).
    pub prefill_tps_per_user: crate::stats::DistSummary,
    /// Per-request `ttft_s + itl_s[0]`; streaming successes with two or more content chunks.
    pub time_to_second_token_s: crate::stats::DistSummary,
    /// `$ / 1M output tokens` from declared price and stage token rate; null when absent.
    pub cost_per_million_output_tokens: Option<f64>,
    /// Client-observed outstanding concurrency vs stage cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_concurrency: Option<crate::concurrency::ObservedConcurrency>,
    /// Time-weighted blocks over stage successes (#195), same definitions as `summary.v3`.
    #[serde(flatten)]
    pub time_weighted: crate::time_weighted::TimeWeightedMetrics,
    pub connect_s: crate::stats::DistSummary,
    pub prefill_s: crate::stats::DistSummary,
    pub decode_s: crate::stats::DistSummary,
    pub decode_tok_s: crate::stats::DistSummary,
    /// Type-7 TTFT distribution over measured successes with a recorded TTFT.
    #[serde(default)]
    pub ttft_s: crate::stats::DistSummary,
    /// Send to response headers over measured successes.
    pub first_byte_s: crate::stats::DistSummary,
    /// Scheduled-to-send delay; `n = 0` for closed-loop stages (not applicable).
    pub queue_delay_s: crate::stats::DistSummary,
    /// Send to first reasoning chunk; streaming thinking models only.
    pub first_reasoning_s: crate::stats::DistSummary,
    /// DNS lookup inside the connector over measured successes (#194).
    #[serde(default)]
    pub dns_s: crate::stats::DistSummary,
    /// Response headers to last body chunk over measured successes (#194).
    #[serde(default)]
    pub receive_s: crate::stats::DistSummary,
    /// Request body bytes per measured success (#194).
    #[serde(default)]
    pub bytes_sent: crate::stats::DistSummary,
    /// Response body bytes per measured success (#194).
    #[serde(default)]
    pub bytes_received: crate::stats::DistSummary,
    /// Response body chunks per measured success (#194).
    #[serde(default)]
    pub chunks_received: crate::stats::DistSummary,
    /// Measured successes on a pooled connection; null when no row carries the flag.
    #[serde(default)]
    pub connections_reused: Option<usize>,
    /// `connections_reused` over successes carrying the flag; null when none do.
    #[serde(default)]
    pub connection_reuse_rate: Option<f64>,
    /// Per-request input tokens from server `usage` (no tokenizer fallback).
    pub isl_tokens: crate::stats::DistSummary,
    /// Per-request output tokens from server `usage` (no tokenizer fallback).
    pub osl_tokens: crate::stats::DistSummary,
    /// Per-request server-reported reasoning tokens; rows without the field are skipped.
    pub reasoning_tokens: crate::stats::DistSummary,
    /// Sum of reported reasoning tokens over stage successes; null when `n = 0`.
    pub reasoning_tokens_total: Option<u64>,
    /// Per-request `output_tokens - reasoning_tokens` for reasoning-reporting rows.
    pub visible_completion_tokens: crate::stats::DistSummary,
    /// Sum of `visible_completion_tokens`; null when `n = 0`.
    pub visible_completion_tokens_total: Option<u64>,
    /// Count of measured successes whose TTFT came from HTTP time-to-first-byte.
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub ttft_approx_count: usize,
    /// Human-readable note when TTFT was approximated or left unmeasured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttft_warning: Option<String>,
    /// Runtime ISL/OSL vs optional targets for this stage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isl_osl: Option<crate::isl_osl::IslOslValidation>,
    /// Modality sweeps only (#197): one distribution per `modality_metrics`
    /// key over measured successes, with the modality binaries' key names
    /// (VLM `image_count`/`image_bytes`; ASR `wer`/`cer`/`rtfx_client`/
    /// `audio_duration_s`; imagegen `images_requested`/`images_returned`).
    /// Omitted for chat, embeddings and rerank.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub modality_metrics: std::collections::BTreeMap<String, crate::stats::DistSummary>,
    /// `--kind imagegen` only (#197): decoded images and distinct digests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_digests: Option<crate::sweep_modality::ImageDigests>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
}

fn is_zero_usize(value: &usize) -> bool {
    *value == 0
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

/// Time-weighted blocks for one stage. Times are seconds from the first
/// measured send on the [`SendClock`] (monotonic when recorded).
/// The window runs from the first measured send (any outcome) to the latest
/// successful completion, the same rule as `runner::window_seconds_from_records`,
/// so no success is clipped. Token rules match the stage `isl_tokens` /
/// `osl_tokens` (server `usage`, rows reporting none skipped).
fn stage_time_weighted(
    measured: &[&BenchRecord],
    successes: &[&BenchRecord],
    generates_output: bool,
) -> crate::time_weighted::TimeWeightedMetrics {
    let Some(clock) = SendClock::new(measured) else {
        return crate::time_weighted::TimeWeightedMetrics::default();
    };
    let spans: Vec<crate::time_weighted::RequestSpan> = successes
        .iter()
        .map(|r| {
            let has_tokens = r.input_tokens > 0 || r.output_tokens > 0;
            crate::time_weighted::RequestSpan {
                start_s: clock.send_s(r),
                latency_s: r.service_latency_s,
                prefill_end_s: crate::summary::phase_split_s(
                    r.ttft_s,
                    r.ttft_source,
                    r.first_reasoning_s,
                ),
                input_tokens: has_tokens.then_some(r.input_tokens),
                output_tokens: (has_tokens && generates_output).then_some(r.output_tokens),
            }
        })
        .collect();
    // The stage window behind `throughput` (#224); spans start at its start.
    let window = if spans.is_empty() {
        0.0
    } else {
        stage_window_seconds(measured.iter().copied()).unwrap_or(0.0)
    };
    crate::time_weighted::compute(&spans, 0.0, window)
}

/// Stage window in seconds behind every stage rate (`throughput`, `goodput`,
/// the token rates): the earliest measured send (any outcome) to the latest
/// successful completion, the same rule as the time-weighted blocks and
/// `runner::window_seconds_from_records`. Taking min and max over the rows,
/// not the first and last spawned, keeps an earlier request that finishes
/// last inside the window (#224). A stage with no success ends at the latest
/// completion of any outcome. Warmup rows are ignored. `None` when the stage
/// has no measured rows.
pub fn stage_window_seconds<'a>(records: impl IntoIterator<Item = &'a BenchRecord>) -> Option<f64> {
    let measured: Vec<&BenchRecord> = records.into_iter().filter(|r| !r.warmup).collect();
    let clock = SendClock::new(&measured)?;
    // Non-finite or negative latency (a hand-edited `compare` CSV) counts as 0.
    let end_of = |r: &&BenchRecord| {
        let latency = r.service_latency_s;
        let latency = if latency.is_finite() {
            latency.max(0.0)
        } else {
            0.0
        };
        clock.send_s(r) + latency
    };
    let end = measured
        .iter()
        .filter(|r| r.success)
        .map(end_of)
        .max_by(f64::total_cmp)
        .or_else(|| measured.iter().map(end_of).max_by(f64::total_cmp))?;
    Some(end.max(f64::EPSILON))
}

/// Send times for one stage, in seconds from its earliest measured send.
/// Uses the monotonic `send_offset_s` when every measured row has a finite
/// one, so a wall-clock (NTP) step inside a stage cannot stretch the window.
/// CSVs written before #224 lack the column and fall back to wall-clock
/// `sent_unix_ns`, subtracted in `u128` so epoch nanoseconds keep precision.
struct SendClock {
    monotonic_origin: Option<f64>,
    wall_origin: u128,
}

impl SendClock {
    /// `None` when there are no rows.
    fn new(measured: &[&BenchRecord]) -> Option<Self> {
        let wall_origin = measured.iter().map(|r| r.sent_unix_ns).min()?;
        let monotonic_origin = measured
            .iter()
            .map(|r| r.send_offset_s.filter(|offset| offset.is_finite()))
            .collect::<Option<Vec<f64>>>()
            .and_then(|offsets| offsets.into_iter().min_by(f64::total_cmp));
        Some(Self {
            monotonic_origin,
            wall_origin,
        })
    }

    fn send_s(&self, record: &BenchRecord) -> f64 {
        match (self.monotonic_origin, record.send_offset_s) {
            (Some(origin), Some(offset)) => (offset - origin).max(0.0),
            _ => record.sent_unix_ns.saturating_sub(self.wall_origin) as f64 / 1e9,
        }
    }
}

pub fn summarize_stage(
    load: f64,
    records: &[BenchRecord],
    seconds: f64,
    slos: &crate::summary::SloConfig,
    config: Option<Value>,
) -> SweepPoint {
    summarize_stage_with_options(load, records, seconds, slos, config, None, None, None, true)
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
        true,
    )
}

/// Full stage summary with optional price, observed concurrency, and ISL/OSL.
///
/// `generates_output` is false for embeddings and rerank stages: they produce
/// no output tokens, so `osl_tokens` stays `n = 0` instead of a run of zeros.
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
    generates_output: bool,
) -> SweepPoint {
    // Warmup rows stay in the CSV for audit but never enter knee / HTML aggregates.
    let measured: Vec<&BenchRecord> = records.iter().filter(|record| !record.warmup).collect();
    let success_rows: Vec<&BenchRecord> = measured
        .iter()
        .copied()
        .filter(|record| record.success)
        .collect();
    let success_lats: Vec<f64> = success_rows.iter().map(|record| record.latency_s).collect();
    // HTTP phase trace (#194): rows without the trace contribute no sample.
    let trace_dist = |pick: fn(&BenchRecord) -> Option<f64>| {
        crate::stats::DistSummary::from_values(
            &success_rows
                .iter()
                .filter_map(|r| pick(r))
                .collect::<Vec<_>>(),
        )
    };
    let reuse_flags: Vec<bool> = success_rows
        .iter()
        .filter_map(|record| record.connection_reused)
        .collect();
    let reused = reuse_flags.iter().filter(|&&flag| flag).count();
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
    let ttft: Vec<f64> = success_rows
        .iter()
        .filter_map(|record| record.ttft_s)
        .collect();
    let first_byte: Vec<f64> = success_rows
        .iter()
        .filter_map(|record| record.first_byte_s)
        .collect();
    // Closed-loop sends set scheduled == sent by construction, so the queue
    // delay is not measured there. Any row that differs marks an open-loop stage.
    let open_loop = measured
        .iter()
        .any(|record| record.scheduled_unix_ns != record.sent_unix_ns);
    let queue_delay: Vec<f64> = if open_loop {
        success_rows
            .iter()
            .map(|record| record.queue_delay_s)
            .collect()
    } else {
        Vec::new()
    };
    let first_reasoning: Vec<f64> = success_rows
        .iter()
        .filter_map(|record| record.first_reasoning_s)
        .collect();
    // Rows with no reported usage are skipped, never counted as zero-token
    // requests. Rerank `input_tokens` is `usage.total_tokens` (all input).
    let token_rows: Vec<&&BenchRecord> = success_rows
        .iter()
        .filter(|record| record.input_tokens > 0 || record.output_tokens > 0)
        .collect();
    let isl: Vec<f64> = token_rows
        .iter()
        .map(|record| record.input_tokens as f64)
        .collect();
    let osl: Vec<f64> = if generates_output {
        token_rows
            .iter()
            .map(|record| record.output_tokens as f64)
            .collect()
    } else {
        Vec::new()
    };
    // A field no row reported (sum 0) is not applicable, never a 0 total;
    // same rule as the modality summary.
    let field_total = |pick: fn(&BenchRecord) -> u64| {
        crate::summary::total(&token_rows.iter().map(|r| pick(r)).collect::<Vec<_>>())
            .filter(|&tokens| tokens > 0)
    };
    let prompt_tokens_total = field_total(|r| r.input_tokens);
    let completion_tokens_total = if generates_output {
        field_total(|r| r.output_tokens)
    } else {
        None
    };
    let input_tokens_per_second = prompt_tokens_total.map(|t| t as f64 / elapsed);
    let total_tokens_per_second = match (prompt_tokens_total, completion_tokens_total) {
        (Some(p), Some(c)) => Some(p.saturating_add(c) as f64 / elapsed),
        _ => None,
    };
    let prefill_rates: Vec<f64> = success_rows
        .iter()
        .filter(|record| !crate::summary::ttft_is_first_byte_approx(record.ttft_source))
        .filter_map(|record| {
            match crate::summary::first_generated_token_s(record.ttft_s, record.first_reasoning_s) {
                Some(first) if record.input_tokens > 0 && first > 0.0 => {
                    Some(record.input_tokens as f64 / first)
                }
                _ => None,
            }
        })
        .collect();
    let second_token: Vec<f64> = success_rows
        .iter()
        .filter_map(|record| Some(record.ttft_s? + record.itl_s.first()?))
        .collect();
    let reasoning: Vec<u64> = success_rows
        .iter()
        .filter_map(|record| record.reasoning_tokens)
        .collect();
    let visible: Vec<u64> = success_rows
        .iter()
        .filter_map(|record| {
            crate::usage::visible_completion_tokens(record.output_tokens, record.reasoning_tokens)
        })
        .collect();
    let ttft_approx_count = success_rows
        .iter()
        .filter(|record| {
            matches!(
                record.ttft_source,
                Some(crate::measurement::TtftSource::FirstByteApprox)
            )
        })
        .count();
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
    let time_weighted = stage_time_weighted(&measured, &success_rows, generates_output);
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
        prompt_tokens_total,
        completion_tokens_total,
        input_tokens_per_second,
        total_tokens_per_second,
        prefill_tps_per_user: crate::stats::DistSummary::from_values(&prefill_rates),
        time_to_second_token_s: crate::stats::DistSummary::from_values(&second_token),
        cost_per_million_output_tokens,
        observed_concurrency,
        time_weighted,
        connect_s: crate::stats::DistSummary::from_values(&connect),
        prefill_s: crate::stats::DistSummary::from_values(&prefill),
        decode_s: crate::stats::DistSummary::from_values(&decode),
        decode_tok_s: crate::stats::DistSummary::from_values(&decode_tok),
        ttft_s: crate::stats::DistSummary::from_values(&ttft),
        first_byte_s: crate::stats::DistSummary::from_values(&first_byte),
        queue_delay_s: crate::stats::DistSummary::from_values(&queue_delay),
        first_reasoning_s: crate::stats::DistSummary::from_values(&first_reasoning),
        dns_s: trace_dist(|r| r.dns_s),
        receive_s: trace_dist(|r| r.receive_s),
        bytes_sent: trace_dist(|r| r.bytes_sent.map(|b| b as f64)),
        bytes_received: trace_dist(|r| r.bytes_received.map(|b| b as f64)),
        chunks_received: trace_dist(|r| r.chunks_received.map(|c| c as f64)),
        connections_reused: (!reuse_flags.is_empty()).then_some(reused),
        connection_reuse_rate: (!reuse_flags.is_empty())
            .then(|| reused as f64 / reuse_flags.len() as f64),
        isl_tokens: crate::stats::DistSummary::from_values(&isl),
        osl_tokens: crate::stats::DistSummary::from_values(&osl),
        reasoning_tokens: crate::stats::DistSummary::from_values(
            &reasoning.iter().map(|&v| v as f64).collect::<Vec<_>>(),
        ),
        reasoning_tokens_total: crate::summary::total(&reasoning),
        visible_completion_tokens: crate::stats::DistSummary::from_values(
            &visible.iter().map(|&v| v as f64).collect::<Vec<_>>(),
        ),
        visible_completion_tokens_total: crate::summary::total(&visible),
        ttft_approx_count,
        ttft_warning: None,
        isl_osl,
        modality_metrics: std::collections::BTreeMap::new(),
        image_digests: None,
        config,
    }
}

/// Minimum number of measured sweep stages (stages with a p95) before a knee
/// is reported: both endpoints plus at least 3 interior candidates. With
/// fewer, Kneedle has at most two interior candidates, so a 3-stage sweep
/// would always return its middle stage.
pub const KNEE_MIN_POINTS: usize = 5;

/// Minimum relative p95 rise before a latency bend counts (0.20 = +20%). The
/// rise is the largest p95 rise over the running minimum,
/// `max_j (p95_j / min_{i<=j} p95_i - 1)`, so a cold first stage, or a bend
/// that recovers by the last stage, is not missed (#232). Kneedle always
/// returns the interior stage farthest from the chord, so without a minimum
/// bend a nearly linear sweep still yields a knee. A chord-distance threshold
/// cannot separate the live H100 curves: the LLM sweep with a real knee at
/// c=32 (+41% p95) peaks at 0.095, below the bend-free VLM sweep (+11% p95,
/// peak 0.164). Provisional: the value sits about 2x above the bend-free
/// sweeps seen so far (+8% to +11%) and about 2x below the smallest real
/// bend (+41%), from two live curves.
pub const KNEE_MIN_P95_RISE: f64 = 0.20;

/// Concurrency sweeps only: a stage is saturated when its relative throughput
/// gain is below this fraction of its relative load gain,
/// `(X_i - X_{i-1}) / X_{i-1} < 0.5 * (load_i - load_{i-1}) / load_{i-1}`,
/// where `X` is success throughput (#232). On 2x steps that is a throughput
/// ratio below 1.5. Load shedding (fast 429/503, admission control) keeps
/// success p95 flat while throughput plateaus, so p95 alone misses it. Rate
/// sweeps skip this check: their stage window ends at the latest completion
/// (#224), so latency spread, not saturation, moves achieved throughput.
pub const KNEE_MIN_MARGINAL_GAIN: f64 = 0.5;

/// Error-rate rise (absolute, 0.05 = 5 points) over the lowest earlier stage
/// at which a stage counts as saturated (#232). The only saturation check on
/// rate sweeps.
pub const KNEE_MAX_ERROR_RATE_RISE: f64 = 0.05;

/// What the sweep `load` axis means, for the saturation checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KneeLoadAxis {
    /// `load` is the stage concurrency cap.
    #[default]
    Concurrency,
    /// `load` is the offered request rate (req/s).
    Rate,
}

/// Why [`detect_knee_with_reason`] reported no knee.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KneeReason {
    /// Fewer than [`KNEE_MIN_POINTS`] measured stages (stages with a p95).
    InsufficientPoints,
    /// The first or last stage has no p95 latency (for example, no
    /// successes), and neither a p95 bend nor a saturation check gave a knee.
    MissingLatency,
    /// Throughput or p95 latency does not change across the measured
    /// stages, or p95 bends over flat throughput, so the curve cannot be
    /// normalized.
    FlatCurve,
    /// p95 latency rises less than [`KNEE_MIN_P95_RISE`] (relative) over its
    /// running minimum, and no saturation check fired, so the curve has no
    /// meaningful bend.
    NoBend,
}

/// How a reported knee was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KneeMethod {
    /// Kneedle chord distance on the throughput/p95 curve from the p95
    /// baseline stage to the p95 peak.
    Kneedle,
    /// The stage before the first saturated stage. Used when p95 has no
    /// bend, or when it comes before the Kneedle knee.
    Saturation,
}

/// Knee detection outcome, serialized into the strategic summary as
/// `knee_detection`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct KneeDetection {
    /// Index into the sweep points, or `None` when there is no knee.
    pub index: Option<usize>,
    /// Set exactly when `index` is `None`.
    pub reason: Option<KneeReason>,
    /// Number of measured sweep stages (stages with a p95).
    pub points: usize,
    /// Always [`KNEE_MIN_POINTS`].
    pub min_points: usize,
    /// How the knee was found; `None` when there is no knee.
    pub method: Option<KneeMethod>,
    /// Largest p95 rise over the running minimum; `None` below
    /// [`KNEE_MIN_POINTS`] or when no measured p95 is positive.
    pub p95_rise: Option<f64>,
    /// First stage index flagged saturated, if any.
    pub saturated_index: Option<usize>,
    /// Always [`KNEE_MIN_P95_RISE`].
    pub min_p95_rise: f64,
    /// [`KNEE_MIN_MARGINAL_GAIN`] on concurrency sweeps; `None` on rate
    /// sweeps, where the check does not run.
    pub min_marginal_gain: Option<f64>,
    /// Always [`KNEE_MAX_ERROR_RATE_RISE`].
    pub max_error_rate_rise: f64,
}

impl KneeDetection {
    fn new(axis: KneeLoadAxis, points: usize) -> Self {
        Self {
            index: None,
            reason: None,
            points,
            min_points: KNEE_MIN_POINTS,
            method: None,
            p95_rise: None,
            saturated_index: None,
            min_p95_rise: KNEE_MIN_P95_RISE,
            min_marginal_gain: (axis == KneeLoadAxis::Concurrency)
                .then_some(KNEE_MIN_MARGINAL_GAIN),
            max_error_rate_rise: KNEE_MAX_ERROR_RATE_RISE,
        }
    }

    fn none(mut self, reason: KneeReason) -> Self {
        self.index = None;
        self.method = None;
        self.reason = Some(reason);
        self
    }

    fn knee(mut self, index: usize, method: KneeMethod) -> Self {
        self.index = Some(index);
        self.method = Some(method);
        self.reason = None;
        self
    }

    /// Human-readable reason for a missing knee; `None` when a knee exists.
    pub fn note(&self) -> Option<String> {
        let reason = self.reason?;
        Some(match reason {
            KneeReason::InsufficientPoints => format!(
                "no knee: {} measured sweep stage(s) (stages with a p95), knee detection needs at least {}",
                self.points, self.min_points
            ),
            KneeReason::MissingLatency => {
                "no knee: the first or last sweep stage has no p95 latency".to_string()
            }
            KneeReason::FlatCurve => {
                "no knee: throughput or p95 latency is flat across the sweep".to_string()
            }
            KneeReason::NoBend => format!(
                "no knee: p95 latency rises less than {:.0}% above its running minimum and no stage is saturated",
                self.min_p95_rise * 100.0
            ),
        })
    }
}

/// Finds the maximum distance from the chord after normalizing the
/// throughput/latency curve. This is the standard deterministic Kneedle
/// construction and is robust to units and uneven sweep spacing. Returns no
/// knee below [`KNEE_MIN_POINTS`] measured stages (stages with a p95) or when
/// the sweep has neither a p95 bend nor a saturated stage; see
/// [`detect_knee_with_reason`].
pub fn detect_knee(points: &[SweepPoint]) -> Option<usize> {
    detect_knee_with_reason(points).index
}

/// [`detect_knee`] plus the machine-readable reason when there is no knee,
/// for a concurrency sweep.
pub fn detect_knee_with_reason(points: &[SweepPoint]) -> KneeDetection {
    detect_knee_on_axis(points, KneeLoadAxis::Concurrency)
}

/// First saturated stage (#232): on concurrency sweeps a marginal throughput
/// gain below [`KNEE_MIN_MARGINAL_GAIN`] of the marginal load gain; on any
/// sweep an error rate at least [`KNEE_MAX_ERROR_RATE_RISE`] above the lowest
/// earlier stage. Never the first stage.
fn first_saturated_stage(points: &[SweepPoint], axis: KneeLoadAxis) -> Option<usize> {
    let mut min_error_rate = points.first().and_then(|point| point.error_rate);
    for (index, pair) in points.windows(2).enumerate() {
        let (previous, point) = (&pair[0], &pair[1]);
        let index = index + 1;
        // One-wave stages (`load >= n`) cannot scale throughput.
        let one_wave = previous.load >= previous.n as f64 && point.load >= point.n as f64;
        if axis == KneeLoadAxis::Concurrency
            && !one_wave
            && previous.throughput > 0.0
            && previous.load > 0.0
            && point.load > previous.load
        {
            let gain = (point.throughput - previous.throughput) / previous.throughput;
            let load_gain = (point.load - previous.load) / previous.load;
            if gain < KNEE_MIN_MARGINAL_GAIN * load_gain {
                return Some(index);
            }
        }
        if let Some(rate) = point.error_rate {
            if min_error_rate.is_some_and(|floor| rate - floor >= KNEE_MAX_ERROR_RATE_RISE) {
                return Some(index);
            }
            min_error_rate = Some(min_error_rate.map_or(rate, |floor| floor.min(rate)));
        }
    }
    None
}

/// Kneedle over `(index, throughput, p95)`, normalized by the data range.
/// Callers pass the curve from the p95 baseline to the p95 peak. Returns the
/// interior entry farthest from the normalized diagonal.
fn kneedle(curve: &[(usize, f64, f64)]) -> Option<usize> {
    if curve.len() < 3 {
        return None;
    }
    let range = |values: &mut dyn Iterator<Item = f64>| {
        values.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(v), hi.max(v))
        })
    };
    let (x_min, x_max) = range(&mut curve.iter().map(|p| p.1));
    let (y_min, y_max) = range(&mut curve.iter().map(|p| p.2));
    if x_max - x_min <= f64::EPSILON || y_max - y_min <= f64::EPSILON {
        return None;
    }
    curve[1..curve.len() - 1]
        .iter()
        .map(|&(index, x, y)| {
            let x = (x - x_min) / (x_max - x_min);
            let y = (y - y_min) / (y_max - y_min);
            (index, (y - x).abs())
        })
        .max_by(|left, right| left.1.total_cmp(&right.1))
        .map(|pair| pair.0)
}

/// Largest p95 rise over the running minimum, as positions into `curve`:
/// `(baseline, peak, rise)`. `None` when no p95 is positive.
fn largest_p95_rise(curve: &[(usize, f64, f64)]) -> Option<(usize, usize, f64)> {
    let mut best: Option<(usize, usize, f64)> = None;
    let mut low: Option<usize> = None;
    for (position, point) in curve.iter().enumerate() {
        if point.2 > 0.0 && low.is_none_or(|low| point.2 < curve[low].2) {
            low = Some(position);
        }
        if let Some(low) = low {
            let rise = point.2 / curve[low].2 - 1.0;
            if best.is_none_or(|(_, _, top)| rise > top) {
                best = Some((low, position, rise));
            }
        }
    }
    best
}

/// [`detect_knee_with_reason`] with the meaning of `load` stated.
///
/// Two candidates: Kneedle from the p95 baseline to the peak of the largest
/// rise over the running minimum, when that rise is at least
/// [`KNEE_MIN_P95_RISE`]; and the stage before the first saturated stage.
/// The earlier one is the knee. Otherwise the reason is `missing_latency`
/// (an endpoint has no p95), `flat_curve`, or `no_bend`.
pub fn detect_knee_on_axis(points: &[SweepPoint], axis: KneeLoadAxis) -> KneeDetection {
    // Stages without a p95 are skipped, so the minimum counts measured
    // stages only: otherwise gaps could leave one candidate, which would
    // always be returned.
    let curve: Vec<(usize, f64, f64)> = points
        .iter()
        .enumerate()
        .filter_map(|(index, point)| point.p95_s.map(|p95| (index, point.throughput, p95)))
        .collect();
    let mut detection = KneeDetection::new(axis, curve.len());
    if curve.len() < KNEE_MIN_POINTS {
        return detection.none(KneeReason::InsufficientPoints);
    }
    detection.saturated_index = first_saturated_stage(points, axis);
    let saturation_knee = detection.saturated_index.map(|index| index - 1);

    let rise = largest_p95_rise(&curve);
    detection.p95_rise = rise.map(|(_, _, rise)| rise);
    let mut bend_over_flat_throughput = false;
    let kneedle_knee = match rise {
        // Multiplicative form, so exactly +20% counts despite rounding.
        Some((base, peak, _)) if curve[peak].2 >= curve[base].2 * (1.0 + KNEE_MIN_P95_RISE) => {
            match kneedle(&curve[base..=peak]) {
                Some(index) => Some(index),
                // Peak right after the baseline: the baseline is the last
                // stage before the rise.
                None if peak == base + 1 => Some(curve[base].0),
                None => {
                    bend_over_flat_throughput = true;
                    None
                }
            }
        }
        _ => None,
    };
    // Earlier wins, so shedding ahead of a later p95 bend is reported where
    // it starts, and a later all-failure stage cannot override a bend.
    match (kneedle_knee, saturation_knee) {
        (Some(k), Some(s)) if s < k => return detection.knee(s, KneeMethod::Saturation),
        (Some(k), _) => return detection.knee(k, KneeMethod::Kneedle),
        (None, Some(s)) => return detection.knee(s, KneeMethod::Saturation),
        (None, None) => {}
    }
    let endpoints = (points.first(), points.last());
    if !matches!(endpoints, (Some(first), Some(last)) if first.p95_s.is_some() && last.p95_s.is_some())
    {
        return detection.none(KneeReason::MissingLatency);
    }
    let flat = |values: &mut dyn Iterator<Item = f64>| {
        let (lo, hi) = values.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(v), hi.max(v))
        });
        hi - lo <= f64::EPSILON
    };
    if bend_over_flat_throughput
        || flat(&mut curve.iter().map(|p| p.1))
        || flat(&mut curve.iter().map(|p| p.2))
    {
        return detection.none(KneeReason::FlatCurve);
    }
    detection.none(KneeReason::NoBend)
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
    knee: &KneeDetection,
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
                if knee.index == Some(index) {
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
        .index
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
    let knee_note = knee
        .note()
        .map(|note| format!("<p>{}.</p>", escape_html(&note)))
        .unwrap_or_default();
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
{knee_note}
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

    fn knee_point(throughput: f64, latency: f64) -> SweepPoint {
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
            prompt_tokens_total: None,
            completion_tokens_total: None,
            input_tokens_per_second: None,
            total_tokens_per_second: None,
            prefill_tps_per_user: crate::stats::DistSummary::from_values(&[]),
            time_to_second_token_s: crate::stats::DistSummary::from_values(&[]),
            cost_per_million_output_tokens: None,
            observed_concurrency: None,
            time_weighted: Default::default(),
            connect_s: crate::stats::DistSummary::from_values(&[]),
            prefill_s: crate::stats::DistSummary::from_values(&[]),
            decode_s: crate::stats::DistSummary::from_values(&[]),
            decode_tok_s: crate::stats::DistSummary::from_values(&[]),
            ttft_s: crate::stats::DistSummary::from_values(&[]),
            first_byte_s: crate::stats::DistSummary::from_values(&[]),
            queue_delay_s: crate::stats::DistSummary::from_values(&[]),
            first_reasoning_s: crate::stats::DistSummary::from_values(&[]),
            dns_s: crate::stats::DistSummary::from_values(&[]),
            receive_s: crate::stats::DistSummary::from_values(&[]),
            bytes_sent: crate::stats::DistSummary::from_values(&[]),
            bytes_received: crate::stats::DistSummary::from_values(&[]),
            chunks_received: crate::stats::DistSummary::from_values(&[]),
            connections_reused: None,
            connection_reuse_rate: None,
            isl_tokens: crate::stats::DistSummary::from_values(&[]),
            osl_tokens: crate::stats::DistSummary::from_values(&[]),
            reasoning_tokens: crate::stats::DistSummary::from_values(&[]),
            reasoning_tokens_total: None,
            visible_completion_tokens_total: None,
            visible_completion_tokens: crate::stats::DistSummary::from_values(&[]),
            ttft_approx_count: 0,
            ttft_warning: None,
            isl_osl: None,
            modality_metrics: std::collections::BTreeMap::new(),
            image_digests: None,
            config: None,
        }
    }

    fn knee_points(pairs: &[(f64, f64)]) -> Vec<SweepPoint> {
        pairs
            .iter()
            .map(|&(throughput, latency)| knee_point(throughput, latency))
            .collect()
    }

    /// Sweep points with an explicit load axis, as (throughput, p95) pairs,
    /// 64 measured requests per stage.
    fn loaded_points(loads: &[f64], pairs: &[(f64, f64)]) -> Vec<SweepPoint> {
        loads
            .iter()
            .zip(knee_points(pairs))
            .map(|(&load, mut point)| {
                point.load = load;
                point.n = 64;
                point
            })
            .collect()
    }

    #[test]
    fn knee_throughput_plateau_with_flat_p95_and_rising_failures() {
        // #232 load shedding: fast 429/503 keep success p95 flat while
        // success throughput plateaus. Pre-fix this reported no_bend.
        let mut points = loaded_points(
            &[1.0, 2.0, 4.0, 8.0, 16.0],
            &[
                (1.0, 1.0),
                (2.0, 1.01),
                (4.0, 1.02),
                (4.1, 1.02),
                (4.0, 1.03),
            ],
        );
        for (point, rate) in points.iter_mut().zip([0.0, 0.0, 0.0, 0.2, 0.5]) {
            point.error_rate = Some(rate);
        }
        let detection = detect_knee_with_reason(&points);
        assert_eq!(detection.reason, None);
        assert_eq!(detection.method, Some(KneeMethod::Saturation));
        // c=8 both fails requests and gains 2.5% throughput for 2x load.
        assert_eq!(detection.saturated_index, Some(3));
        assert_eq!(detection.index, Some(2));
    }

    #[test]
    fn knee_flat_plateau_without_errors_is_saturated() {
        // Admission control that queues instead of failing: no errors, flat
        // success p95, throughput stops at c=8 (ratio 1.05 < 1.5 on a 2x step).
        let points = loaded_points(
            &[1.0, 2.0, 4.0, 8.0, 16.0],
            &[
                (1.0, 1.0),
                (2.0, 1.0),
                (4.0, 1.01),
                (4.2, 1.01),
                (4.2, 1.02),
            ],
        );
        let detection = detect_knee_with_reason(&points);
        assert_eq!(detection.saturated_index, Some(3));
        assert_eq!(
            (detection.index, detection.method),
            (Some(2), Some(KneeMethod::Saturation))
        );
        assert_eq!(detection.min_marginal_gain, Some(KNEE_MIN_MARGINAL_GAIN));
    }

    #[test]
    fn knee_rate_sweep_saturates_on_errors_only() {
        // Rate stage windows end at the latest completion (#224), so achieved
        // throughput lags offered on healthy servers; it is not checked.
        let mut points = loaded_points(
            &[1.0, 2.0, 4.0, 8.0, 16.0],
            &[
                (0.9, 10.0),
                (1.7, 10.1),
                (2.9, 10.2),
                (3.5, 10.3),
                (3.7, 10.4),
            ],
        );
        let detection = detect_knee_on_axis(&points, KneeLoadAxis::Rate);
        assert_eq!(detection.saturated_index, None);
        assert_eq!(detection.reason, Some(KneeReason::NoBend));
        assert_eq!(detection.min_marginal_gain, None);
        for (point, rate) in points.iter_mut().zip([0.0, 0.0, 0.0, 0.0, 0.08]) {
            point.error_rate = Some(rate);
        }
        let detection = detect_knee_on_axis(&points, KneeLoadAxis::Rate);
        assert_eq!(detection.saturated_index, Some(4));
        assert_eq!(
            (detection.index, detection.method),
            (Some(3), Some(KneeMethod::Saturation))
        );
    }

    #[test]
    fn knee_error_rate_gate_alone() {
        // Throughput scales and p95 is flat; only failures rise at c=8.
        let mut points = loaded_points(
            &[1.0, 2.0, 4.0, 8.0, 16.0],
            &[
                (1.0, 1.0),
                (2.0, 1.0),
                (4.0, 1.01),
                (8.0, 1.01),
                (16.0, 1.02),
            ],
        );
        for (point, rate) in points.iter_mut().zip([0.01, 0.0, 0.02, 0.06, 0.1]) {
            point.error_rate = Some(rate);
        }
        let detection = detect_knee_with_reason(&points);
        assert_eq!(detection.saturated_index, Some(3));
        assert_eq!(
            (detection.index, detection.method),
            (Some(2), Some(KneeMethod::Saturation))
        );
    }

    #[test]
    fn knee_saturation_before_bend_wins() {
        // Shedding starts at c=8 (errors +10 points), p95 bends later at
        // c=32. Kneedle alone would report c=16.
        let mut points = loaded_points(
            &[1.0, 2.0, 4.0, 8.0, 16.0, 32.0],
            &[
                (1.0, 1.0),
                (2.0, 1.0),
                (4.0, 1.01),
                (7.0, 1.02),
                (12.0, 1.05),
                (14.0, 2.0),
            ],
        );
        for (point, rate) in points.iter_mut().zip([0.0, 0.0, 0.0, 0.1, 0.2, 0.3]) {
            point.error_rate = Some(rate);
        }
        let detection = detect_knee_with_reason(&points);
        assert_eq!(detection.saturated_index, Some(3));
        assert_eq!(
            (detection.index, detection.method),
            (Some(2), Some(KneeMethod::Saturation))
        );
        // Without the failures the Kneedle knee stands.
        for point in &mut points {
            point.error_rate = Some(0.0);
        }
        let detection = detect_knee_with_reason(&points);
        assert_eq!(detection.method, Some(KneeMethod::Kneedle));
        assert_eq!(detection.index, Some(4));
    }

    #[test]
    fn knee_missing_endpoint_with_saturation_reports_saturation() {
        // The last stage has no successes: no p95, error rate 1.
        let mut points = loaded_points(
            &[1.0, 2.0, 4.0, 8.0, 16.0, 32.0],
            &[
                (1.0, 1.0),
                (2.0, 1.0),
                (4.0, 1.0),
                (8.0, 1.01),
                (16.0, 1.02),
                (0.0, 1.0),
            ],
        );
        points[5].p95_s = None;
        points[5].error_rate = Some(1.0);
        let detection = detect_knee_with_reason(&points);
        assert_eq!(detection.saturated_index, Some(5));
        assert_eq!(
            (detection.index, detection.method),
            (Some(4), Some(KneeMethod::Saturation))
        );
    }

    #[test]
    fn knee_later_all_failure_stage_does_not_override_earlier_bend() {
        // p95 bends at c=8..16, then c=32 fails every request (no p95).
        let mut points = loaded_points(
            &[1.0, 2.0, 4.0, 8.0, 16.0, 32.0],
            &[
                (1.0, 1.0),
                (2.0, 1.02),
                (4.0, 1.04),
                (7.0, 1.6),
                (8.0, 2.0),
                (0.0, 1.0),
            ],
        );
        points[5].p95_s = None;
        points[5].error_rate = Some(1.0);
        let detection = detect_knee_with_reason(&points);
        assert_eq!(
            (detection.index, detection.method),
            (Some(2), Some(KneeMethod::Kneedle))
        );
    }

    #[test]
    fn knee_baseline_right_before_peak_is_the_knee() {
        // p95 falls to its minimum at stage 3 and jumps at the last stage.
        let points = loaded_points(
            &[1.0, 2.0, 4.0, 8.0, 16.0],
            &[(1.0, 1.5), (2.0, 1.4), (4.0, 1.2), (8.0, 1.0), (14.0, 1.6)],
        );
        let detection = detect_knee_with_reason(&points);
        assert_eq!(
            (detection.index, detection.method),
            (Some(3), Some(KneeMethod::Kneedle))
        );
    }

    #[test]
    fn knee_cold_first_stage_does_not_hide_the_bend() {
        // Stage 0 is inflated (cold), then p95 rises +50% from its minimum.
        // Pre-fix the last/first ratio was 0.5, so this reported no_bend.
        let points = loaded_points(
            &[1.0, 2.0, 4.0, 8.0, 16.0],
            &[
                (1.0, 3.0),
                (2.0, 1.0),
                (4.0, 1.02),
                (8.0, 1.05),
                (12.0, 1.5),
            ],
        );
        let detection = detect_knee_with_reason(&points);
        assert_eq!(detection.reason, None);
        assert_eq!(detection.method, Some(KneeMethod::Kneedle));
        assert_eq!(detection.index, Some(3));
        let rise = detection.p95_rise.expect("p95 rise");
        assert!((rise - 0.5).abs() < 1e-9, "{rise}");
    }

    #[test]
    fn knee_mid_sweep_bend_with_last_stage_drop() {
        // p95 bends at c=8..16 and the last stage drops below the first
        // (orchestrator review curve). The global minimum is the last stage,
        // so a min-then-later-max rule found no rise; the running minimum
        // finds +100% from stage 0 to stage 4.
        let points = loaded_points(
            &[1.0, 2.0, 4.0, 8.0, 16.0, 32.0],
            &[
                (1.0, 1.0),
                (2.0, 1.02),
                (4.0, 1.04),
                (7.0, 1.6),
                (8.0, 2.0),
                (8.5, 0.99),
            ],
        );
        let detection = detect_knee_with_reason(&points);
        assert_eq!(detection.reason, None);
        assert_eq!(detection.method, Some(KneeMethod::Kneedle));
        assert_eq!(detection.index, Some(2));
        let rise = detection.p95_rise.expect("p95 rise");
        assert!((rise - 1.0).abs() < 1e-9, "{rise}");
    }

    #[test]
    fn knee_finds_curve_bend() {
        // Flat latency, then a sharp bend after stage 3 (stage 4 is saturated).
        let points = knee_points(&[
            (1.0, 1.0),
            (2.0, 1.02),
            (3.0, 1.04),
            (4.0, 1.06),
            (4.2, 4.0),
        ]);
        assert_eq!(detect_knee(&points), Some(3));
        let detection = detect_knee_with_reason(&points);
        assert_eq!(detection.index, Some(3));
        assert_eq!(detection.reason, None);
        assert_eq!(detection.points, 5);
        assert_eq!(detection.min_points, KNEE_MIN_POINTS);
    }

    #[test]
    fn knee_five_points_picks_correct_stage() {
        // Latency bends at stage 2; stages 3 and 4 are past saturation.
        let points = knee_points(&[(1.0, 1.0), (2.0, 1.1), (3.0, 1.2), (3.1, 3.0), (3.15, 5.0)]);
        assert_eq!(detect_knee(&points), Some(2));
    }

    #[test]
    fn knee_three_points_reports_insufficient_points() {
        let points = knee_points(&[(1.0, 1.0), (2.0, 1.1), (2.1, 4.0)]);
        let detection = detect_knee_with_reason(&points);
        assert_eq!(detection.index, None);
        assert_eq!(detection.reason, Some(KneeReason::InsufficientPoints));
        assert_eq!(detection.points, 3);
        assert_eq!(detection.min_points, 5);
        assert_eq!(detect_knee(&points), None);
    }

    #[test]
    fn knee_four_points_reports_insufficient_points() {
        // The pre-#190 fixture: used to return Some(2).
        let points = knee_points(&[(1.0, 1.0), (2.0, 1.1), (3.0, 1.3), (3.2, 4.0)]);
        let detection = detect_knee_with_reason(&points);
        assert_eq!(detection.index, None);
        assert_eq!(detection.reason, Some(KneeReason::InsufficientPoints));
        assert_eq!(detection.points, 4);
        assert_eq!(detect_knee(&points), None);
    }

    #[test]
    fn knee_reasons_for_flat_and_missing_latency() {
        let flat = knee_points(&[(1.0, 1.0), (2.0, 1.0), (3.0, 1.0), (4.0, 1.0), (5.0, 1.0)]);
        assert_eq!(
            detect_knee_with_reason(&flat).reason,
            Some(KneeReason::FlatCurve)
        );
        // Five measured stages, no bend, no saturation, and no last-stage
        // p95: no curve endpoint.
        let mut missing = knee_points(&[
            (1.0, 1.0),
            (2.0, 1.02),
            (3.0, 1.03),
            (4.0, 1.04),
            (5.0, 1.05),
            (6.0, 6.0),
        ]);
        missing[5].p95_s = None;
        let detection = detect_knee_with_reason(&missing);
        assert_eq!(detection.reason, Some(KneeReason::MissingLatency));
        assert_eq!(detection.points, 5);
        // Only 4 measured stages: too few, whichever stage lost its p95.
        let mut short = knee_points(&[(1.0, 1.0), (2.0, 1.1), (3.0, 1.2), (4.0, 2.0), (5.0, 4.0)]);
        short[4].p95_s = None;
        let detection = detect_knee_with_reason(&short);
        assert_eq!(detection.reason, Some(KneeReason::InsufficientPoints));
        assert_eq!(detection.points, 4);
        let mut interior =
            knee_points(&[(1.0, 1.0), (2.0, 1.1), (3.0, 1.2), (4.0, 2.0), (5.0, 4.0)]);
        for point in &mut interior[1..4] {
            point.p95_s = None;
        }
        let detection = detect_knee_with_reason(&interior);
        assert_eq!(detection.reason, Some(KneeReason::InsufficientPoints));
        assert_eq!(detection.points, 2);
        // A single interior gap is skipped when 5 measured stages remain.
        let mut gap = knee_points(&[
            (1.0, 1.0),
            (2.0, 1.02),
            (3.0, 1.04),
            (3.5, 1.05),
            (4.0, 1.06),
            (4.2, 4.0),
        ]);
        gap[1].p95_s = None;
        let detection = detect_knee_with_reason(&gap);
        assert_eq!(detection.index, Some(4));
        assert_eq!(detection.reason, None);
        assert_eq!(detection.points, 5);
    }

    #[test]
    fn knee_five_stages_with_two_interior_gaps_reports_insufficient_points() {
        // Pre-fix: one interior candidate left, always returned as the knee.
        let mut points = knee_points(&[(1.0, 1.0), (2.0, 1.1), (3.0, 1.2), (4.0, 2.0), (5.0, 4.0)]);
        points[1].p95_s = None;
        points[3].p95_s = None;
        let detection = detect_knee_with_reason(&points);
        assert_eq!(detection.index, None);
        assert_eq!(detection.reason, Some(KneeReason::InsufficientPoints));
        assert_eq!(detection.points, 3);
        assert_eq!(detection.min_points, KNEE_MIN_POINTS);
    }

    /// Live H100 sweeps from the #184 validation (vLLM 0.31.0), as
    /// (throughput req/s, p95 s).
    const LIVE_LLM_SWEEP: [(f64, f64); 7] = [
        (1.7826, 0.63232),
        (3.5073, 0.6436),
        (6.9881, 0.650015),
        (13.357, 0.669489),
        (25.5903, 0.688941),
        (50.0576, 0.752006),
        (88.5565, 0.892026),
    ];
    const LIVE_VLM_SWEEP: [(f64, f64); 5] = [
        (0.9401, 1.200315),
        (1.758, 1.231624),
        (3.4628, 1.233315),
        (6.6041, 1.245416),
        (12.3657, 1.333327),
    ];

    #[test]
    fn knee_live_vlm_nearly_linear_sweep_reports_no_bend() {
        // Pre-#232 this returned index 1 (c=2) on a +11% p95 rise.
        // Real loads c=1..16: every 2x step gains 1.87x to 1.97x throughput
        // (above 1.5), so the saturation gate stays quiet too.
        let points = loaded_points(&[1.0, 2.0, 4.0, 8.0, 16.0], &LIVE_VLM_SWEEP);
        let detection = detect_knee_with_reason(&points);
        assert_eq!(detection.index, None);
        assert_eq!(detection.reason, Some(KneeReason::NoBend));
        assert_eq!(detection.saturated_index, None);
        let rise = detection.p95_rise.expect("p95 rise");
        assert!((rise - 0.1108).abs() < 1e-3, "{rise}");
        assert_eq!(detection.points, 5);
        assert_eq!(detection.min_points, KNEE_MIN_POINTS);
        let note = detection.note().expect("no-knee note");
        assert!(note.contains("20%"), "{note}");
    }

    #[test]
    fn knee_live_llm_sweep_keeps_knee_at_c32() {
        let concurrency = [1, 2, 4, 8, 16, 32, 64];
        let loads = concurrency.map(f64::from);
        let detection = detect_knee_with_reason(&loaded_points(&loads, &LIVE_LLM_SWEEP));
        assert_eq!(detection.reason, None);
        assert_eq!(detection.method, Some(KneeMethod::Kneedle));
        assert_eq!(detection.saturated_index, None);
        let index = detection.index.expect("knee");
        assert_eq!(concurrency[index], 32);
    }

    #[test]
    fn knee_min_p95_rise_boundary_and_falling_curve() {
        // Same interior shape; only the last-stage p95 moves across +20%.
        let below = knee_points(&[(1.0, 1.0), (2.0, 1.0), (3.0, 1.0), (4.0, 1.0), (4.2, 1.19)]);
        assert_eq!(
            detect_knee_with_reason(&below).reason,
            Some(KneeReason::NoBend)
        );
        // Exactly +20% is not "less than 20%", so it keeps the knee.
        let at = knee_points(&[(1.0, 1.0), (2.0, 1.0), (3.0, 1.0), (4.0, 1.0), (4.2, 1.2)]);
        assert_eq!(detect_knee_with_reason(&at).index, Some(3));
        let above = knee_points(&[(1.0, 1.0), (2.0, 1.0), (3.0, 1.0), (4.0, 1.0), (4.2, 1.21)]);
        assert_eq!(detect_knee_with_reason(&above).index, Some(3));
        let falling = knee_points(&[(1.0, 2.0), (2.0, 1.8), (3.0, 1.6), (4.0, 1.2), (5.0, 1.0)]);
        assert_eq!(
            detect_knee_with_reason(&falling).reason,
            Some(KneeReason::NoBend)
        );
        assert_eq!(
            serde_json::to_value(KneeReason::NoBend).expect("serialize"),
            serde_json::json!("no_bend")
        );
    }

    #[test]
    fn knee_detection_serializes_snake_case_reason() {
        let detection = detect_knee_with_reason(&knee_points(&[(1.0, 1.0)]));
        assert_eq!(
            serde_json::to_value(detection).expect("serialize"),
            serde_json::json!({
                "index": null,
                "reason": "insufficient_points",
                "points": 1,
                "min_points": 5,
                "method": null,
                "p95_rise": null,
                "saturated_index": null,
                "min_p95_rise": 0.2,
                "min_marginal_gain": 0.5,
                "max_error_rate_rise": 0.05
            })
        );
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
            prompt_tokens_total: None,
            completion_tokens_total: None,
            input_tokens_per_second: None,
            total_tokens_per_second: None,
            prefill_tps_per_user: crate::stats::DistSummary::from_values(&[]),
            time_to_second_token_s: crate::stats::DistSummary::from_values(&[]),
            cost_per_million_output_tokens: None,
            observed_concurrency: None,
            time_weighted: Default::default(),
            connect_s: crate::stats::DistSummary::from_values(&[]),
            prefill_s: crate::stats::DistSummary::from_values(&[]),
            decode_s: crate::stats::DistSummary::from_values(&[]),
            decode_tok_s: crate::stats::DistSummary::from_values(&[]),
            ttft_s: crate::stats::DistSummary::from_values(&[]),
            first_byte_s: crate::stats::DistSummary::from_values(&[]),
            queue_delay_s: crate::stats::DistSummary::from_values(&[]),
            first_reasoning_s: crate::stats::DistSummary::from_values(&[]),
            dns_s: crate::stats::DistSummary::from_values(&[]),
            receive_s: crate::stats::DistSummary::from_values(&[]),
            bytes_sent: crate::stats::DistSummary::from_values(&[]),
            bytes_received: crate::stats::DistSummary::from_values(&[]),
            chunks_received: crate::stats::DistSummary::from_values(&[]),
            connections_reused: None,
            connection_reuse_rate: None,
            isl_tokens: crate::stats::DistSummary::from_values(&[]),
            osl_tokens: crate::stats::DistSummary::from_values(&[]),
            reasoning_tokens: crate::stats::DistSummary::from_values(&[]),
            reasoning_tokens_total: None,
            visible_completion_tokens_total: None,
            visible_completion_tokens: crate::stats::DistSummary::from_values(&[]),
            ttft_approx_count: 0,
            ttft_warning: None,
            isl_osl: None,
            modality_metrics: std::collections::BTreeMap::new(),
            image_digests: None,
            config: None,
        }];
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.html");
        export_html(
            &path,
            "unit test",
            &points,
            &detect_knee_with_reason(&points),
            &ServerMetrics::default(),
            None,
        )
        .unwrap();
        let html = std::fs::read_to_string(&path).unwrap();
        assert!(html.contains("Throughput (req/s)"));
        assert!(html.contains("p95 latency (s)"));
        assert!(html.contains("text-anchor"));
        assert!(html.contains("<polyline"));
        assert!(html.contains("no knee: 1 measured sweep stage(s) (stages with a p95), knee detection needs at least 5."));
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
    fn summarize_stage_reports_recorded_field_dists() {
        let base = BenchRecord {
            seq: 0,
            stage: 2.0,
            endpoint: "http://example.test".to_string(),
            scheduled_unix_ns: 5,
            sent_unix_ns: 5,
            latency_s: 0.5,
            queue_delay_s: 0.0,
            service_latency_s: 0.5,
            first_byte_s: Some(0.04),
            connect_s: None,
            ttft_s: Some(0.1),
            ttft_source: None,
            prefill_s: None,
            decode_s: None,
            decode_tok_s: None,
            itl_s: Vec::new(),
            in_flight_at_send: None,
            success: true,
            valid: None,
            input_tokens: 64,
            output_tokens: 16,
            session_id: None,
            turn: None,
            error: None,
            warmup: false,
            first_reasoning_s: Some(0.06),
            reasoning_tokens: None,
            connection_reused: None,
            dns_s: None,
            bytes_sent: None,
            receive_s: None,
            bytes_received: None,
            chunks_received: None,
            send_offset_s: None,
        };
        let slos = crate::summary::SloConfig::default();
        // Closed loop: scheduled == sent on every row, queue delay not applicable.
        let closed = summarize_stage(2.0, &[base.clone(), base.clone()], 1.0, &slos, None);
        assert_eq!(closed.first_byte_s.n, 2);
        assert_eq!(closed.first_reasoning_s.n, 2);
        assert_eq!(closed.isl_tokens.avg, Some(64.0));
        assert_eq!(closed.osl_tokens.avg, Some(16.0));
        assert_eq!(closed.queue_delay_s.n, 0);
        // HTTP phase trace (#194): rows without the trace give n = 0 and null.
        assert_eq!(closed.receive_s.n, 0);
        assert_eq!(closed.connections_reused, None);
        assert_eq!(closed.connection_reuse_rate, None);
        let mut traced = base.clone();
        traced.connection_reused = Some(false);
        traced.dns_s = Some(0.001);
        traced.bytes_sent = Some(120);
        traced.receive_s = Some(0.3);
        traced.bytes_received = Some(900);
        traced.chunks_received = Some(9);
        let mut pooled = traced.clone();
        pooled.connection_reused = Some(true);
        pooled.dns_s = Some(0.0);
        let trace_point = summarize_stage(2.0, &[traced, pooled], 1.0, &slos, None);
        assert_eq!(trace_point.connections_reused, Some(1));
        assert_eq!(trace_point.connection_reuse_rate, Some(0.5));
        assert_eq!(trace_point.bytes_received.avg, Some(900.0));
        assert_eq!(trace_point.chunks_received.n, 2);
        assert_eq!(trace_point.bytes_sent.n, 2);
        assert_eq!(trace_point.receive_s.n, 2);
        assert_eq!(trace_point.dns_s.n, 2);
        // Open loop: a delayed send marks the stage, every success contributes.
        let mut late = base.clone();
        late.sent_unix_ns = 3_000_005;
        late.queue_delay_s = 0.003;
        let mut no_usage = base.clone();
        no_usage.input_tokens = 0;
        no_usage.output_tokens = 0;
        no_usage.first_byte_s = None;
        no_usage.first_reasoning_s = None;
        let open = summarize_stage(2.0, &[base, late, no_usage], 1.0, &slos, None);
        assert_eq!(open.queue_delay_s.n, 3);
        assert_eq!(open.isl_tokens.n, 2);
        assert_eq!(open.first_byte_s.n, 2);
        assert_eq!(open.first_reasoning_s.n, 2);
    }

    #[test]
    fn non_generating_stage_has_no_osl_tokens() {
        // Embeddings shape: input usage reported, no output tokens.
        let row = BenchRecord {
            seq: 0,
            stage: 1.0,
            endpoint: "http://example.test/v1/embeddings".to_string(),
            scheduled_unix_ns: 5,
            sent_unix_ns: 5,
            latency_s: 0.05,
            queue_delay_s: 0.0,
            service_latency_s: 0.05,
            first_byte_s: Some(0.04),
            connect_s: None,
            ttft_s: None,
            ttft_source: None,
            prefill_s: None,
            decode_s: None,
            decode_tok_s: None,
            itl_s: Vec::new(),
            in_flight_at_send: None,
            success: true,
            valid: None,
            input_tokens: 12,
            output_tokens: 0,
            session_id: None,
            turn: None,
            error: None,
            warmup: false,
            first_reasoning_s: None,
            reasoning_tokens: None,
            connection_reused: None,
            dns_s: None,
            bytes_sent: None,
            receive_s: None,
            bytes_received: None,
            chunks_received: None,
            send_offset_s: None,
        };
        let point = summarize_stage_with_options(
            1.0,
            &[row.clone(), row],
            1.0,
            &crate::summary::SloConfig::default(),
            None,
            None,
            None,
            None,
            false,
        );
        assert_eq!(point.osl_tokens.n, 0);
        assert!(point.osl_tokens.avg.is_none());
        assert_eq!(point.isl_tokens.n, 2);
        assert_eq!(point.isl_tokens.avg, Some(12.0));
        // No output tokens: input totals still report, total rate does not.
        assert_eq!(point.prompt_tokens_total, Some(24));
        assert!(point.input_tokens_per_second.is_some());
        assert!(point.completion_tokens_total.is_none());
        assert!(point.total_tokens_per_second.is_none());
    }

    #[test]
    fn summarize_stage_token_totals_and_rates_match_rows() {
        let base = BenchRecord {
            seq: 0,
            stage: 2.0,
            endpoint: "http://example.test".to_string(),
            scheduled_unix_ns: 5,
            sent_unix_ns: 5,
            latency_s: 0.5,
            queue_delay_s: 0.0,
            service_latency_s: 0.5,
            first_byte_s: None,
            connect_s: None,
            ttft_s: Some(0.1),
            ttft_source: None,
            prefill_s: None,
            decode_s: None,
            decode_tok_s: None,
            itl_s: vec![0.02, 0.03],
            in_flight_at_send: None,
            success: true,
            valid: None,
            input_tokens: 40,
            output_tokens: 20,
            session_id: None,
            turn: None,
            error: None,
            warmup: false,
            first_reasoning_s: None,
            reasoning_tokens: None,
            connection_reused: None,
            dns_s: None,
            bytes_sent: None,
            receive_s: None,
            bytes_received: None,
            chunks_received: None,
            send_offset_s: None,
        };
        let mut second = base.clone();
        second.seq = 1;
        second.ttft_s = Some(0.2);
        second.itl_s = vec![0.05];
        second.input_tokens = 100;
        second.output_tokens = 30;
        second.service_latency_s = 1.0;
        // Unary row: no TTFT or ITL, so no prefill rate or second token.
        let mut unary = base.clone();
        unary.seq = 2;
        unary.ttft_s = None;
        unary.itl_s = Vec::new();
        // Failed rows never count.
        let mut failed = base.clone();
        failed.seq = 3;
        failed.success = false;
        let rows = [base.clone(), second.clone(), unary.clone(), failed];
        let window = 2.0;
        let point = summarize_stage(
            2.0,
            &rows,
            window,
            &crate::summary::SloConfig::default(),
            None,
        );
        let ok = [&base, &second, &unary];
        let prompt: u64 = ok.iter().map(|r| r.input_tokens).sum();
        let completion: u64 = ok.iter().map(|r| r.output_tokens).sum();
        assert_eq!(point.prompt_tokens_total, Some(prompt));
        assert_eq!(point.completion_tokens_total, Some(completion));
        assert_eq!(point.input_tokens_per_second, Some(prompt as f64 / window));
        assert_eq!(
            point.total_tokens_per_second,
            Some((prompt + completion) as f64 / window)
        );
        assert_eq!(
            point.completion_tokens_per_second,
            Some(completion as f64 / window)
        );
        assert_eq!(point.prefill_tps_per_user.n, 2);
        let prefill_mean = (40.0 / 0.1 + 100.0 / 0.2) / 2.0;
        assert!((point.prefill_tps_per_user.avg.unwrap() - prefill_mean).abs() < 1e-9);
        // A reasoning row ends prefill at its first reasoning token (#221 review).
        let mut thinking = base.clone();
        thinking.ttft_s = Some(0.4);
        thinking.first_reasoning_s = Some(0.05);
        let slos_default = crate::summary::SloConfig::default();
        let reasoning_point = summarize_stage(1.0, &[thinking], 1.0, &slos_default, None);
        assert_eq!(reasoning_point.prefill_tps_per_user.n, 1);
        assert!((reasoning_point.prefill_tps_per_user.avg.unwrap() - 40.0 / 0.05).abs() < 1e-9);
        assert_eq!(point.time_to_second_token_s.n, 2);
        let ttst_mean = ((0.1 + 0.02) + (0.2 + 0.05)) / 2.0;
        assert!((point.time_to_second_token_s.avg.unwrap() - ttst_mean).abs() < 1e-9);
        assert_eq!(point.user_tps.n, 3);
        let user_mean = (20.0 / 0.5 + 30.0 / 1.0 + 20.0 / 0.5) / 3.0;
        assert!((point.user_tps.avg.unwrap() - user_mean).abs() < 1e-9);
        // No usage on any success: totals and rates are null, not 0.
        let mut bare = base.clone();
        bare.input_tokens = 0;
        bare.output_tokens = 0;
        let empty = summarize_stage(
            1.0,
            &[bare],
            1.0,
            &crate::summary::SloConfig::default(),
            None,
        );
        assert!(empty.prompt_tokens_total.is_none());
        assert!(empty.completion_tokens_total.is_none());
        assert!(empty.input_tokens_per_second.is_none());
        assert!(empty.total_tokens_per_second.is_none());
        assert_eq!(empty.prefill_tps_per_user.n, 0);
        // Usage with output only: the prompt total is not applicable, not 0.
        let mut output_only = base.clone();
        output_only.input_tokens = 0;
        let slos = crate::summary::SloConfig::default();
        let point = summarize_stage(1.0, &[output_only], 1.0, &slos, None);
        assert!(point.prompt_tokens_total.is_none());
        assert!(point.input_tokens_per_second.is_none());
        assert!(point.total_tokens_per_second.is_none());
        assert_eq!(point.completion_tokens_total, Some(20));
        // First-byte approximated TTFT never feeds the prefill rate.
        let mut approx = base.clone();
        approx.ttft_source = Some(crate::measurement::TtftSource::FirstByteApprox);
        let point = summarize_stage(1.0, &[approx], 1.0, &slos, None);
        assert_eq!(point.prefill_tps_per_user.n, 0);
        assert_eq!(point.time_to_second_token_s.n, 1);
    }

    #[test]
    fn summarize_stage_reports_reasoning_tokens_from_reporting_rows() {
        let row = BenchRecord {
            seq: 0,
            stage: 1.0,
            endpoint: "http://example.test".to_string(),
            scheduled_unix_ns: 5,
            sent_unix_ns: 5,
            latency_s: 0.5,
            queue_delay_s: 0.0,
            service_latency_s: 0.5,
            first_byte_s: None,
            connect_s: None,
            ttft_s: None,
            ttft_source: None,
            prefill_s: None,
            decode_s: None,
            decode_tok_s: None,
            itl_s: Vec::new(),
            in_flight_at_send: None,
            success: true,
            valid: None,
            input_tokens: 8,
            output_tokens: 20,
            session_id: None,
            turn: None,
            error: None,
            warmup: false,
            first_reasoning_s: None,
            reasoning_tokens: Some(5),
            connection_reused: None,
            dns_s: None,
            bytes_sent: None,
            receive_s: None,
            bytes_received: None,
            chunks_received: None,
            send_offset_s: None,
        };
        let mut other = row.clone();
        other.reasoning_tokens = Some(7);
        let mut unreported = row.clone();
        unreported.reasoning_tokens = None;
        let slos = crate::summary::SloConfig::default();
        let point = summarize_stage(
            1.0,
            &[row.clone(), other, unreported.clone()],
            1.0,
            &slos,
            None,
        );
        assert_eq!(point.reasoning_tokens.n, 2);
        assert_eq!(point.reasoning_tokens.avg, Some(6.0));
        assert_eq!(point.reasoning_tokens_total, Some(12));
        assert_eq!(point.visible_completion_tokens.n, 2);
        assert_eq!(point.visible_completion_tokens.avg, Some(14.0));
        assert_eq!(point.visible_completion_tokens_total, Some(28));
        // The new column round-trips through CSV; absence stays empty, not 0.
        let mut writer = csv::Writer::from_writer(Vec::new());
        for written in [&row, &unreported] {
            writer.serialize(written).unwrap();
        }
        let bytes = writer.into_inner().unwrap();
        let back: Vec<BenchRecord> = csv::Reader::from_reader(bytes.as_slice())
            .deserialize()
            .collect::<std::result::Result<_, _>>()
            .expect("csv round trip");
        assert_eq!(back[0].reasoning_tokens, Some(5));
        assert_eq!(back[1].reasoning_tokens, None);
        let empty = summarize_stage(1.0, &back[1..], 1.0, &slos, None);
        assert_eq!(empty.reasoning_tokens.n, 0);
        assert!(empty.reasoning_tokens_total.is_none());
    }

    #[test]
    fn http_trace_columns_round_trip_through_csv() {
        let row = BenchRecord {
            seq: 0,
            stage: 1.0,
            endpoint: "http://example.test".to_string(),
            scheduled_unix_ns: 5,
            sent_unix_ns: 5,
            latency_s: 0.5,
            queue_delay_s: 0.0,
            service_latency_s: 0.5,
            first_byte_s: Some(0.04),
            connect_s: Some(0.003),
            ttft_s: None,
            ttft_source: None,
            prefill_s: None,
            decode_s: None,
            decode_tok_s: None,
            itl_s: Vec::new(),
            in_flight_at_send: None,
            success: true,
            valid: None,
            input_tokens: 8,
            output_tokens: 20,
            session_id: None,
            turn: None,
            error: None,
            warmup: false,
            first_reasoning_s: None,
            reasoning_tokens: None,
            connection_reused: Some(false),
            dns_s: Some(0.001),
            bytes_sent: Some(120),
            receive_s: Some(0.25),
            bytes_received: Some(900),
            chunks_received: Some(9),
            send_offset_s: Some(1.25),
        };
        let mut untraced = row.clone();
        untraced.connection_reused = None;
        untraced.dns_s = None;
        untraced.bytes_sent = None;
        untraced.receive_s = None;
        untraced.bytes_received = None;
        untraced.chunks_received = None;
        let mut writer = csv::Writer::from_writer(Vec::new());
        for written in [&row, &untraced] {
            writer.serialize(written).unwrap();
        }
        let bytes = writer.into_inner().unwrap();
        let text = String::from_utf8(bytes.clone()).expect("utf8");
        assert!(text
            .lines()
            .next()
            .expect("header")
            .ends_with(",reasoning_tokens,connection_reused,dns_s,bytes_sent,receive_s,bytes_received,chunks_received,send_offset_s"));
        let back: Vec<BenchRecord> = csv::Reader::from_reader(bytes.as_slice())
            .deserialize()
            .collect::<std::result::Result<_, _>>()
            .expect("csv round trip");
        assert_eq!(back[0].connection_reused, Some(false));
        assert_eq!(back[0].dns_s, Some(0.001));
        assert_eq!(back[0].bytes_sent, Some(120));
        assert_eq!(back[0].receive_s, Some(0.25));
        assert_eq!(back[0].bytes_received, Some(900));
        assert_eq!(back[0].chunks_received, Some(9));
        // Empty cells read back as absent, not 0 or false.
        assert_eq!(back[1].connection_reused, None);
        assert_eq!(back[1].dns_s, None);
        assert_eq!(back[1].bytes_sent, None);
        assert_eq!(back[1].receive_s, None);
        assert_eq!(back[1].bytes_received, None);
        assert_eq!(back[1].chunks_received, None);
    }

    #[test]
    fn pre_191_csv_without_first_reasoning_column_still_loads() {
        // Header and row as written before #191 (no trailing first_reasoning_s).
        let old = "seq,stage,endpoint,scheduled_unix_ns,sent_unix_ns,latency_s,queue_delay_s,\
service_latency_s,first_byte_s,connect_s,ttft_s,ttft_source,prefill_s,decode_s,decode_tok_s,\
itl_s,in_flight_at_send,success,valid,input_tokens,output_tokens,session_id,turn,error,warmup\n\
0,1.0,http://example.test,5,5,0.5,0.0,0.5,0.04,0.0,0.1,stream,0.1,0.4,40.0,0.01;0.02,1,\
true,,64,16,,,,false\n";
        let records: Vec<BenchRecord> = csv::Reader::from_reader(old.as_bytes())
            .deserialize()
            .collect::<std::result::Result<_, _>>()
            .expect("old CSV deserializes");
        assert_eq!(records.len(), 1);
        assert!(records[0].first_reasoning_s.is_none());
        assert!(records[0].reasoning_tokens.is_none());
        assert!(records[0].send_offset_s.is_none());
        let point = summarize_stage(
            1.0,
            &records,
            1.0,
            &crate::summary::SloConfig::default(),
            None,
        );
        assert_eq!(point.first_reasoning_s.n, 0);
        assert_eq!(point.first_byte_s.n, 1);
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
            ttft_source: None,
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
            first_reasoning_s: None,
            reasoning_tokens: None,
            connection_reused: None,
            dns_s: None,
            bytes_sent: None,
            receive_s: None,
            bytes_received: None,
            chunks_received: None,
            send_offset_s: None,
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
            ttft_source: None,
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
            first_reasoning_s: None,
            reasoning_tokens: None,
            connection_reused: None,
            dns_s: None,
            bytes_sent: None,
            receive_s: None,
            bytes_received: None,
            chunks_received: None,
            send_offset_s: None,
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
            ttft_source: None,
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
            first_reasoning_s: None,
            reasoning_tokens: None,
            connection_reused: None,
            dns_s: None,
            bytes_sent: None,
            receive_s: None,
            bytes_received: None,
            chunks_received: None,
            send_offset_s: None,
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
            ttft_source: None,
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
            first_reasoning_s: None,
            reasoning_tokens: None,
            connection_reused: None,
            dns_s: None,
            bytes_sent: None,
            receive_s: None,
            bytes_received: None,
            chunks_received: None,
            send_offset_s: None,
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
            ttft_source: None,
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
            first_reasoning_s: None,
            reasoning_tokens: None,
            connection_reused: None,
            dns_s: None,
            bytes_sent: None,
            receive_s: None,
            bytes_received: None,
            chunks_received: None,
            send_offset_s: None,
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

    /// #224: the stage window ends at the latest successful completion, not
    /// at the completion of the last-spawned row. Row 0 is sent first and
    /// finishes last, so the old rule (first row send to last row end) gave a
    /// 2 s window and read throughput high.
    #[test]
    fn stage_window_ends_at_latest_completion() {
        let epoch: u128 = 1_790_000_000_000_000_000;
        let row = |seq, send_s: f64, latency: f64, success: bool, warmup: bool| BenchRecord {
            seq,
            stage: 4.0,
            endpoint: "http://example.test".to_string(),
            scheduled_unix_ns: epoch + (send_s * 1e9) as u128,
            sent_unix_ns: epoch + (send_s * 1e9) as u128,
            latency_s: latency,
            queue_delay_s: 0.0,
            service_latency_s: latency,
            first_byte_s: None,
            connect_s: None,
            ttft_s: Some(0.1),
            ttft_source: None,
            prefill_s: None,
            decode_s: None,
            decode_tok_s: None,
            itl_s: Vec::new(),
            in_flight_at_send: None,
            success,
            valid: None,
            input_tokens: 10,
            output_tokens: 20,
            session_id: None,
            turn: None,
            error: None,
            warmup,
            first_reasoning_s: None,
            reasoning_tokens: None,
            connection_reused: None,
            dns_s: None,
            bytes_sent: None,
            receive_s: None,
            bytes_received: None,
            chunks_received: None,
            send_offset_s: None,
        };
        // Spawn order: row 0 (sent 1.5 s, ends 6.5 s), a failure (sent 1 s,
        // ends 10.5 s), row 1 (sent 2.5 s, ends 3.5 s). The window starts at
        // the failure's send, not at the warmup row (sent 0 s, ends 20 s) or
        // the first success, and ends at row 0, not the failure or row 1.
        let rows = [
            row(9, 0.0, 20.0, true, true),
            row(0, 1.5, 5.0, true, false),
            row(2, 1.0, 9.5, false, false),
            row(1, 2.5, 1.0, true, false),
        ];
        let window = stage_window_seconds(&rows).expect("window");
        assert!((window - 5.5).abs() < 1e-9, "{window}");

        // Every stage rate and the time-weighted blocks share that window.
        let point = summarize_stage(
            4.0,
            &rows,
            window,
            &crate::summary::SloConfig::default(),
            None,
        );
        assert!((point.throughput - 2.0 / 5.5).abs() < 1e-9);
        assert!((point.goodput - 2.0 / 5.5).abs() < 1e-9);
        let tokens = point.completion_tokens_per_second.expect("token rate");
        assert!((tokens - 40.0 / 5.5).abs() < 1e-9);
        let avg = point.time_weighted.effective_concurrency.avg.expect("avg");
        assert!((avg - 6.0 / 5.5).abs() < 1e-9, "{avg}");

        // A non-finite latency (hand-edited CSV) counts as 0, no overflow.
        let odd = [
            row(0, 1.0, 2.0, true, false),
            row(1, 1.5, f64::INFINITY, true, false),
        ];
        let window = stage_window_seconds(&odd).expect("window");
        assert!((window - 2.0).abs() < 1e-9, "{window}");

        // No success: the window falls back to the latest completion of any
        // outcome. No measured rows: no window.
        let failed = [
            row(0, 0.0, 3.0, false, false),
            row(1, 1.0, 1.0, false, false),
        ];
        let window = stage_window_seconds(&failed).expect("window");
        assert!((window - 3.0).abs() < 1e-9, "{window}");
        assert_eq!(stage_window_seconds(&[row(9, 0.0, 1.0, true, true)]), None);
        assert_eq!(stage_window_seconds(&[]), None);
    }

    /// #224: a wall-clock step inside a stage does not stretch the window.
    /// The second send is 1 s after the first on the monotonic clock, but an
    /// NTP step moved the wall clock forward 60 s in between.
    #[test]
    fn stage_window_follows_monotonic_send_offset_over_wall_clock_step() {
        let epoch: u128 = 1_790_000_000_000_000_000;
        let row = |seq, wall_s: f64, mono_s: Option<f64>| BenchRecord {
            seq,
            stage: 2.0,
            endpoint: "http://example.test".to_string(),
            scheduled_unix_ns: epoch + (wall_s * 1e9) as u128,
            sent_unix_ns: epoch + (wall_s * 1e9) as u128,
            latency_s: 2.0,
            service_latency_s: 2.0,
            success: true,
            output_tokens: 10,
            send_offset_s: mono_s,
            ..Default::default()
        };
        let stepped = [row(0, 0.0, Some(10.0)), row(1, 61.0, Some(11.0))];
        let window = stage_window_seconds(&stepped).expect("window");
        assert!((window - 3.0).abs() < 1e-9, "{window}");
        let point = summarize_stage(
            2.0,
            &stepped,
            window,
            &crate::summary::SloConfig::default(),
            None,
        );
        assert!((point.throughput - 2.0 / 3.0).abs() < 1e-9);
        // Spans are on the same clock: 4 s of work over a 3 s window.
        let avg = point.time_weighted.effective_concurrency.avg.expect("avg");
        assert!((avg - 4.0 / 3.0).abs() < 1e-9, "{avg}");

        // A row without the column (older CSV) puts the stage on wall clock.
        let mixed = [row(0, 0.0, Some(10.0)), row(1, 61.0, None)];
        let window = stage_window_seconds(&mixed).expect("window");
        assert!((window - 63.0).abs() < 1e-9, "{window}");
    }

    /// #195: sweep points carry the same time-weighted blocks as `summary.v3`
    /// (synthetic intervals from `time_weighted::tests`, epoch nanoseconds).
    #[test]
    fn sweep_point_time_weighted_blocks_from_synthetic_intervals() {
        let epoch: u128 = 1_790_000_000_000_000_000;
        let row = |seq, send_s: f64, latency: f64, ttft: f64, input, output| BenchRecord {
            seq,
            stage: 2.0,
            endpoint: "http://example.test".to_string(),
            scheduled_unix_ns: epoch + (send_s * 1e9) as u128,
            sent_unix_ns: epoch + (send_s * 1e9) as u128,
            latency_s: latency,
            queue_delay_s: 0.0,
            service_latency_s: latency,
            first_byte_s: None,
            connect_s: None,
            ttft_s: Some(ttft),
            ttft_source: None,
            prefill_s: None,
            decode_s: None,
            decode_tok_s: None,
            itl_s: Vec::new(),
            in_flight_at_send: None,
            success: true,
            valid: None,
            input_tokens: input,
            output_tokens: output,
            session_id: None,
            turn: None,
            error: None,
            warmup: false,
            first_reasoning_s: None,
            reasoning_tokens: None,
            connection_reused: None,
            dns_s: None,
            bytes_sent: None,
            receive_s: None,
            bytes_received: None,
            chunks_received: None,
            send_offset_s: None,
        };
        let mut warmup = row(9, 0.0, 3.0, 0.1, 999, 999);
        warmup.warmup = true;
        let rows = [
            row(0, 0.0, 2.0, 0.5, 100, 30),
            row(1, 1.0, 2.0, 1.5, 50, 10),
            warmup,
        ];
        // The stage `seconds` (4.0) is not the time-weighted window: that runs
        // from the first send to the latest completion, 3 s here.
        let point = summarize_stage(2.0, &rows, 4.0, &crate::summary::SloConfig::default(), None);
        let tw = &point.time_weighted;
        let close = |got: Option<f64>, want: f64| {
            let got = got.expect("value");
            assert!((got - want).abs() < 1e-12, "{got} != {want}");
        };
        assert_eq!(tw.effective_concurrency.n, 2);
        close(tw.effective_concurrency.avg, 4.0 / 3.0);
        close(tw.effective_concurrency.max, 2.0);
        close(tw.effective_prefill_concurrency.avg, 2.0 / 3.0);
        close(tw.effective_decode_concurrency.active_avg, 1.0);
        close(tw.tokens_in_flight.avg, 325.0 / 3.0);
        close(tw.effective_prefill_throughput.avg, 50.0);
        close(tw.effective_decode_throughput.avg, 40.0 / 3.0);
        let json = serde_json::to_value(&point).expect("json");
        assert_eq!(json["tokens_in_flight"]["n"], 2);
        assert_eq!(json["effective_decode_throughput"]["max"], 20.0);
    }
}
