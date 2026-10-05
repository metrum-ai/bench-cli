// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::args_common::EffectiveCommonArgs;
use crate::concurrency::ObservedConcurrency;
use crate::isl_osl::IslOslValidation;
use crate::record::{Phase, RequestRecord, SCHEMA_VERSION_SUMMARY};
use crate::stats::{bootstrap_mean_ci, ConfidenceInterval, DistSummary};
use crate::sut::Sut;
use serde::Serialize;
use std::collections::BTreeMap;

/// Effective workload configuration stamped onto `summary.v3`.
#[derive(Debug, Clone, Serialize)]
pub struct EffectiveRunConfig {
    pub run_id: String,
    pub common: EffectiveCommonArgs,
    /// Cap actually used for outstanding requests (`max_concurrency` or
    /// closed-loop `--concurrency` when the former is unset) (F-09).
    pub effective_max_concurrency: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_system_prompt: Option<String>,
    pub body_template: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unique_prompt_nonce_template: Option<String>,
    /// Modality-specific fields (e.g. ASR `normalizer`, VLM `reencode_jpeg`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub modality: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunSummary {
    pub schema_version: &'static str,
    pub attempted: usize,
    pub successes: usize,
    pub errors: usize,
    pub error_rate: f64,
    pub errors_by_type: BTreeMap<String, usize>,
    pub window_seconds: f64,
    pub requests_per_second: f64,
    pub usage_missing_count: usize,
    pub completion_tokens_per_second: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens_source: Option<&'static str>,
    /// Sum of prompt tokens over measured successes (server `usage`, tokenizer
    /// fills `usage_missing` rows); null when no row reports tokens or a gap is unfilled.
    pub prompt_tokens_total: Option<u64>,
    /// Sum of completion tokens behind `completion_tokens_per_second`; null when that is null.
    pub completion_tokens_total: Option<u64>,
    /// `prompt_tokens_total / window_seconds`; null when the total is null.
    pub input_tokens_per_second: Option<f64>,
    /// `(prompt_tokens_total + completion_tokens_total) / window_seconds`; null unless both totals exist.
    pub total_tokens_per_second: Option<f64>,
    pub latency_s: DistSummary,
    pub coordinated_omission_latency_s: DistSummary,
    pub ttft_s: DistSummary,
    pub tpot_s: DistSummary,
    pub itl_s: DistSummary,
    /// TCP/TLS connector duration (`0` = pool hit).
    pub connect_s: DistSummary,
    /// Prefill proxy (`ttft - connect` when connect present, else `ttft`).
    pub prefill_s: DistSummary,
    /// Decode proxy (`e2e - ttft`).
    pub decode_s: DistSummary,
    /// Completion tokens per decode second.
    pub decode_tok_s: DistSummary,
    /// Send to response headers (HTTP time to first byte).
    pub first_byte_s: DistSummary,
    /// Intended arrival to actual send; open-loop successes only (`n = 0` closed loop).
    pub queue_delay_s: DistSummary,
    /// Send to first reasoning chunk; streaming thinking models only.
    pub first_reasoning_s: DistSummary,
    /// Per-request prefill rate `isl_tokens / ttft_s`; rows with both values
    /// and visible-token TTFT (first-byte approximations excluded).
    pub prefill_tps_per_user: DistSummary,
    /// Per-request `ttft_s + itl_s[0]`; streaming rows with at least two content chunks.
    pub time_to_second_token_s: DistSummary,
    /// Per-request `completion_tokens / latency_s` (same definition as the `user_tps=` SLO).
    pub user_tps: DistSummary,
    /// Per-request input tokens (server `usage`, tokenizer fills `usage_missing` rows).
    pub isl_tokens: DistSummary,
    /// Provenance of `isl_tokens`: `server_usage`, `tokenizer_fallback`, or `mixed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isl_tokens_source: Option<&'static str>,
    /// Per-request output tokens (server `usage`, tokenizer fills `usage_missing` rows).
    pub osl_tokens: DistSummary,
    /// Provenance of `osl_tokens`: `server_usage`, `tokenizer_fallback`, or `mixed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub osl_tokens_source: Option<&'static str>,
    /// Per-request server-reported reasoning tokens (rows that reported none are skipped).
    pub reasoning_tokens: DistSummary,
    /// Sum of `reasoning_tokens` over measured successes that reported it; null when `n = 0`.
    pub reasoning_tokens_total: Option<u64>,
    /// Per-request `completion_tokens - reasoning_tokens` (reasoning-reporting rows only).
    pub visible_completion_tokens: DistSummary,
    /// Sum of `visible_completion_tokens`; null when `n = 0`.
    pub visible_completion_tokens_total: Option<u64>,
    pub throughput_bins_rps: DistSummary,
    pub goodput: GoodputSummary,
    pub pooled_mixture: bool,
    pub per_endpoint: BTreeMap<String, EndpointSummary>,
    pub environment: serde_json::Value,
    /// Operator-declared system under test. Always present; `null` when absent.
    pub sut: Option<Sut>,
    /// Declared `$ / hour` used for cost (CLI or SUT); null when absent.
    pub price_per_hour: Option<f64>,
    /// Provenance of `price_per_hour`: `"cli"` or `"sut"`.
    pub price_provenance: Option<&'static str>,
    /// `$ / 1M output tokens` from price and measured output tok/s; null when absent.
    pub cost_per_million_output_tokens: Option<f64>,
    /// Client-observed outstanding concurrency vs configured cap.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_concurrency: Option<ObservedConcurrency>,
    /// Runtime ISL/OSL vs optional targets; omitted when no targets configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isl_osl: Option<IslOslValidation>,
    /// Count of measured successes whose TTFT came from HTTP time-to-first-byte.
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub ttft_approx_count: usize,
    /// Human-readable note when TTFT was approximated or left unmeasured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttft_warning: Option<String>,
    pub partial: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<EffectiveRunConfig>,
}

fn is_zero_usize(value: &usize) -> bool {
    *value == 0
}

#[derive(Debug, Clone, Default)]
pub struct SloConfig {
    pub ttft_s: Option<f64>,
    pub tpot_s: Option<f64>,
    pub e2e_s: Option<f64>,
    /// Minimum output tokens/second per in-flight user (higher is better).
    pub user_tps: Option<f64>,
}

impl SloConfig {
    pub fn parse(values: &[String]) -> anyhow::Result<Self> {
        let mut config = Self::default();
        for value in values {
            let (name, raw) = value
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("SLO must be METRIC=VALUE: {value}"))?;
            match name {
                "ttft" => config.ttft_s = Some(parse_duration_seconds(raw)?),
                "tpot" => config.tpot_s = Some(parse_duration_seconds(raw)?),
                "e2e" | "latency" => config.e2e_s = Some(parse_duration_seconds(raw)?),
                "user_tps" => {
                    let rate: f64 = raw
                        .trim()
                        .parse()
                        .map_err(|e| anyhow::anyhow!("user_tps rate must be a number: {e}"))?;
                    if !rate.is_finite() || rate < 0.0 {
                        anyhow::bail!("user_tps must be finite and non-negative");
                    }
                    config.user_tps = Some(rate);
                }
                _ => anyhow::bail!("unknown SLO metric '{name}'"),
            }
        }
        Ok(config)
    }
}

/// `$ / 1M output tokens` = `price_per_hour / (tok_per_s * 3600) * 1e6`.
///
/// Returns `None` when token throughput is missing, zero, or non-finite.
pub fn cost_per_million_output_tokens(price_per_hour: f64, toks_per_sec: f64) -> Option<f64> {
    if !price_per_hour.is_finite() || price_per_hour < 0.0 {
        return None;
    }
    if !toks_per_sec.is_finite() || toks_per_sec <= 0.0 {
        return None;
    }
    Some(price_per_hour / (toks_per_sec * 3600.0) * 1_000_000.0)
}

/// Resolve declared hourly price: CLI `--price-per-hour` wins over `sut.cost`.
pub fn resolve_price_per_hour(
    cli_price_per_hour: Option<f64>,
    sut: Option<&Sut>,
) -> Option<(f64, &'static str)> {
    if let Some(price) = cli_price_per_hour {
        return Some((price, "cli"));
    }
    sut.and_then(|s| s.cost.as_ref())
        .and_then(|c| c.price_per_hour)
        .map(|price| (price, "sut"))
}

fn parse_duration_seconds(raw: &str) -> anyhow::Result<f64> {
    let (number, scale) = if let Some(v) = raw.strip_suffix("ms") {
        (v, 0.001)
    } else if let Some(v) = raw.strip_suffix('s') {
        (v, 1.0)
    } else {
        (raw, 1.0)
    };
    let value: f64 = number.parse()?;
    if !value.is_finite() || value < 0.0 {
        anyhow::bail!("SLO duration must be finite and non-negative");
    }
    Ok(value * scale)
}

#[derive(Debug, Clone, Serialize)]
pub struct GoodputSummary {
    pub count: usize,
    pub requests_per_second: f64,
    pub fraction_of_attempted: f64,
    pub thresholds_s: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EndpointSummary {
    pub attempted: usize,
    pub successes: usize,
    pub errors: usize,
    pub latency_s: DistSummary,
    pub ttft_s: DistSummary,
    pub tpot_s: DistSummary,
    pub itl_s: DistSummary,
    pub first_byte_s: DistSummary,
    pub queue_delay_s: DistSummary,
    pub first_reasoning_s: DistSummary,
    pub isl_tokens: DistSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isl_tokens_source: Option<&'static str>,
    pub osl_tokens: DistSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub osl_tokens_source: Option<&'static str>,
    pub reasoning_tokens: DistSummary,
    pub visible_completion_tokens: DistSummary,
}

/// Distributions over fields already recorded per request (#191).
struct RecordedFieldDists {
    first_byte_s: DistSummary,
    queue_delay_s: DistSummary,
    first_reasoning_s: DistSummary,
    prefill_tps_per_user: DistSummary,
    time_to_second_token_s: DistSummary,
    user_tps: DistSummary,
    isl_tokens: DistSummary,
    isl_tokens_source: Option<&'static str>,
    osl_tokens: DistSummary,
    osl_tokens_source: Option<&'static str>,
    reasoning_tokens: DistSummary,
    reasoning_tokens_total: Option<u64>,
    visible_completion_tokens: DistSummary,
    visible_completion_tokens_total: Option<u64>,
}

impl RecordedFieldDists {
    /// Build from successful records. Never fabricates a value: absent
    /// timings, closed-loop queue delay, and rows with no token accounting
    /// (ASR, imagegen) contribute no sample.
    fn from_successes(successes: &[&RequestRecord]) -> Self {
        let first_byte: Vec<f64> = successes.iter().filter_map(|r| r.first_byte_s).collect();
        // Queue delay is only measured when a request had an intended arrival.
        let queue_delay: Vec<f64> = successes
            .iter()
            .filter(|r| r.scheduled_offset_s.is_some())
            .map(|r| r.queue_delay_s)
            .collect();
        let first_reasoning: Vec<f64> = successes
            .iter()
            .filter_map(|r| r.first_reasoning_s)
            .collect();
        let prefill_rates: Vec<f64> = successes
            .iter()
            .filter(|r| !ttft_is_first_byte_approx(r.ttft_source))
            .filter_map(|r| match (record_isl_tokens(r), r.ttft_s) {
                (Some(isl), Some(ttft)) if isl > 0 && ttft > 0.0 => Some(isl as f64 / ttft),
                _ => None,
            })
            .collect();
        let second_token: Vec<f64> = successes
            .iter()
            .filter_map(|r| Some(r.ttft_s? + r.itl_s.first()?))
            .collect();
        let user_rates: Vec<f64> = successes.iter().filter_map(|r| r.user_tps()).collect();
        let isl = token_samples(successes, |r| (r.prompt_tokens, r.tokenized_prompt_tokens));
        let osl = token_samples(successes, |r| {
            (r.completion_tokens, r.tokenized_completion_tokens)
        });
        // Reasoning counts come only from server usage; no tokenizer fallback.
        let reasoning: Vec<u64> = successes
            .iter()
            .filter_map(|r| r.reasoning_tokens)
            .collect();
        let visible: Vec<u64> = successes
            .iter()
            .filter_map(|r| r.visible_completion_tokens)
            .collect();
        Self {
            first_byte_s: DistSummary::from_values(&first_byte),
            queue_delay_s: DistSummary::from_values(&queue_delay),
            first_reasoning_s: DistSummary::from_values(&first_reasoning),
            prefill_tps_per_user: DistSummary::from_values(&prefill_rates),
            time_to_second_token_s: DistSummary::from_values(&second_token),
            user_tps: DistSummary::from_values(&user_rates),
            isl_tokens: DistSummary::from_values(&isl.values),
            isl_tokens_source: isl.source(),
            osl_tokens: DistSummary::from_values(&osl.values),
            osl_tokens_source: osl.source(),
            reasoning_tokens: DistSummary::from_values(&as_f64(&reasoning)),
            reasoning_tokens_total: total(&reasoning),
            visible_completion_tokens: DistSummary::from_values(&as_f64(&visible)),
            visible_completion_tokens_total: total(&visible),
        }
    }
}

fn as_f64(values: &[u64]) -> Vec<f64> {
    values.iter().map(|&v| v as f64).collect()
}

/// Sum of reported counts; `None` (not `0`) when nothing was reported.
/// Saturates so a bogus server count cannot overflow the total.
pub(crate) fn total(values: &[u64]) -> Option<u64> {
    (!values.is_empty()).then(|| values.iter().fold(0u64, |acc, &v| acc.saturating_add(v)))
}

struct TokenSamples {
    values: Vec<f64>,
    server: usize,
    tokenizer: usize,
}

impl TokenSamples {
    fn source(&self) -> Option<&'static str> {
        match (self.server > 0, self.tokenizer > 0) {
            (true, false) => Some("server_usage"),
            (false, true) => Some("tokenizer_fallback"),
            (true, true) => Some("mixed"),
            (false, false) => None,
        }
    }
}

/// First-byte TTFT can land before prefill ends, which would inflate an
/// ISL / TTFT rate, so prefill rates use visible-token TTFT only.
pub(crate) fn ttft_is_first_byte_approx(source: Option<crate::measurement::TtftSource>) -> bool {
    matches!(
        source,
        Some(crate::measurement::TtftSource::FirstByteApprox)
    )
}

/// Prompt tokens for one row with the same rule as `isl_tokens`: server
/// `usage` wins, the tokenizer fills `usage_missing` rows, and a row with no
/// token accounting at all yields `None`.
fn record_isl_tokens(record: &RequestRecord) -> Option<u64> {
    if record.usage_missing {
        record.tokenized_prompt_tokens
    } else if record.prompt_tokens > 0 || record.completion_tokens > 0 || record.total_tokens > 0 {
        Some(record.prompt_tokens)
    } else {
        None
    }
}

/// Per-request token counts. Server `usage` wins; the tokenizer fills only
/// rows flagged `usage_missing`. A row that reports no usage and is not
/// flagged (non-token modalities) is skipped rather than counted as zero.
fn token_samples(
    successes: &[&RequestRecord],
    pick: impl Fn(&RequestRecord) -> (u64, Option<u64>),
) -> TokenSamples {
    let mut samples = TokenSamples {
        values: Vec::new(),
        server: 0,
        tokenizer: 0,
    };
    for record in successes {
        let (server, tokenized) = pick(record);
        if record.usage_missing {
            if let Some(tokens) = tokenized {
                samples.values.push(tokens as f64);
                samples.tokenizer += 1;
            }
        } else if record.prompt_tokens > 0
            || record.completion_tokens > 0
            || record.total_tokens > 0
        {
            samples.values.push(server as f64);
            samples.server += 1;
        }
    }
    samples
}

#[derive(Debug, Clone, Serialize)]
pub struct CrossRunSummary {
    pub runs: usize,
    pub requests_per_second: DistSummary,
    pub requests_per_second_ci95: ConfidenceInterval,
    pub completion_tokens_per_second: DistSummary,
    pub completion_tokens_per_second_ci95: ConfidenceInterval,
}

impl RunSummary {
    /// Summarize measurement-phase records. Warmup/drain excluded from latency
    /// distributions. Error rate uses attempted = successes + errors in the
    /// supplied slice (caller should pass measurement-phase records only, or
    /// all records - attempted is the slice length).
    pub fn from_records(records: &[RequestRecord], window_seconds: f64, partial: bool) -> Self {
        Self::from_records_with_options(
            records,
            window_seconds,
            partial,
            &SloConfig::default(),
            10.0,
        )
    }

    pub fn from_records_with_options(
        records: &[RequestRecord],
        window_seconds: f64,
        partial: bool,
        slos: &SloConfig,
        bin_seconds: f64,
    ) -> Self {
        let measured: Vec<&RequestRecord> = records
            .iter()
            .filter(|r| r.phase == Phase::Measure)
            .collect();
        let attempted = measured.len();
        let pool = measured;
        let successes: Vec<&RequestRecord> =
            pool.iter().copied().filter(|r| r.is_success()).collect();
        let errors: Vec<&RequestRecord> =
            pool.iter().copied().filter(|r| !r.is_success()).collect();
        let mut errors_by_type = BTreeMap::new();
        for e in &errors {
            if let Some(err) = &e.error {
                *errors_by_type
                    .entry(err.type_name().to_string())
                    .or_insert(0) += 1;
            }
        }
        let lat: Vec<f64> = successes.iter().map(|r| r.latency_s).collect();
        let corrected_lat: Vec<f64> = successes.iter().map(|r| r.corrected_latency_s()).collect();
        let ttft: Vec<f64> = successes.iter().filter_map(|r| r.ttft_s).collect();
        let tpot: Vec<f64> = successes.iter().filter_map(|r| r.tpot_s()).collect();
        let itl: Vec<f64> = successes
            .iter()
            .flat_map(|r| r.itl_s.iter().copied())
            .collect();
        let connect: Vec<f64> = successes.iter().filter_map(|r| r.connect_s).collect();
        let prefill: Vec<f64> = successes.iter().filter_map(|r| r.prefill_s).collect();
        let decode: Vec<f64> = successes.iter().filter_map(|r| r.decode_s).collect();
        let decode_tok: Vec<f64> = successes.iter().filter_map(|r| r.decode_tok_s).collect();
        let completion = token_sum_accounting(&successes, |r| {
            (r.completion_tokens, r.tokenized_completion_tokens)
        });
        let prompt =
            token_sum_accounting(&successes, |r| (r.prompt_tokens, r.tokenized_prompt_tokens));
        let usage_missing_count = completion.usage_missing_count;
        let completion_tokens_total = completion.total();
        let prompt_tokens_total = prompt.total();
        let window = if window_seconds > 0.0 {
            window_seconds
        } else {
            1e-9
        };
        let good: Vec<_> = successes
            .iter()
            .filter(|record| meets_slos(record, slos))
            .collect();
        let thresholds_s = {
            let mut map: BTreeMap<String, f64> = [
                ("ttft", slos.ttft_s),
                ("tpot", slos.tpot_s),
                ("e2e", slos.e2e_s),
            ]
            .into_iter()
            .filter_map(|(name, value)| value.map(|v| (name.to_string(), v)))
            .collect();
            if let Some(rate) = slos.user_tps {
                map.insert("user_tps".to_string(), rate);
            }
            map
        };
        let per_endpoint = endpoint_summaries(&pool);
        let recorded = RecordedFieldDists::from_successes(&successes);
        let bins = throughput_bins(&successes, window, bin_seconds);
        let completion_tokens_per_second = completion_tokens_total.map(|t| t as f64 / window);
        let input_tokens_per_second = prompt_tokens_total.map(|t| t as f64 / window);
        let total_tokens_per_second = match (prompt_tokens_total, completion_tokens_total) {
            (Some(p), Some(c)) => Some(p.saturating_add(c) as f64 / window),
            _ => None,
        };
        Self {
            schema_version: SCHEMA_VERSION_SUMMARY,
            attempted,
            successes: successes.len(),
            errors: errors.len(),
            error_rate: if attempted == 0 {
                0.0
            } else {
                errors.len() as f64 / attempted as f64
            },
            errors_by_type,
            window_seconds,
            requests_per_second: successes.len() as f64 / window,
            usage_missing_count,
            completion_tokens_per_second,
            completion_tokens_source: completion.source,
            prompt_tokens_total,
            completion_tokens_total,
            input_tokens_per_second,
            total_tokens_per_second,
            latency_s: DistSummary::from_values(&lat),
            coordinated_omission_latency_s: DistSummary::from_values(&corrected_lat),
            ttft_s: DistSummary::from_values(&ttft),
            tpot_s: DistSummary::from_values(&tpot),
            itl_s: DistSummary::from_values(&itl),
            connect_s: DistSummary::from_values(&connect),
            prefill_s: DistSummary::from_values(&prefill),
            decode_s: DistSummary::from_values(&decode),
            decode_tok_s: DistSummary::from_values(&decode_tok),
            first_byte_s: recorded.first_byte_s,
            queue_delay_s: recorded.queue_delay_s,
            first_reasoning_s: recorded.first_reasoning_s,
            prefill_tps_per_user: recorded.prefill_tps_per_user,
            time_to_second_token_s: recorded.time_to_second_token_s,
            user_tps: recorded.user_tps,
            isl_tokens: recorded.isl_tokens,
            isl_tokens_source: recorded.isl_tokens_source,
            osl_tokens: recorded.osl_tokens,
            osl_tokens_source: recorded.osl_tokens_source,
            reasoning_tokens: recorded.reasoning_tokens,
            reasoning_tokens_total: recorded.reasoning_tokens_total,
            visible_completion_tokens: recorded.visible_completion_tokens,
            visible_completion_tokens_total: recorded.visible_completion_tokens_total,
            throughput_bins_rps: DistSummary::from_values(&bins),
            goodput: GoodputSummary {
                count: good.len(),
                requests_per_second: good.len() as f64 / window,
                fraction_of_attempted: if attempted == 0 {
                    0.0
                } else {
                    good.len() as f64 / attempted as f64
                },
                thresholds_s,
            },
            pooled_mixture: per_endpoint.len() > 1,
            per_endpoint,
            environment: crate::environment::collect(None, None, false),
            sut: None,
            price_per_hour: None,
            price_provenance: None,
            cost_per_million_output_tokens: None,
            observed_concurrency: None,
            isl_osl: None,
            ttft_approx_count: 0,
            ttft_warning: None,
            partial,
            config: None,
        }
    }

    /// Stamp effective run configuration after summary construction.
    pub fn with_config(mut self, config: EffectiveRunConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Embed an operator-declared SUT block (`None` serializes as JSON `null`).
    pub fn with_sut(mut self, sut: Option<Sut>) -> Self {
        self.sut = sut;
        self
    }

    /// Attach client-observed concurrency snapshot.
    pub fn with_observed_concurrency(mut self, observed: Option<ObservedConcurrency>) -> Self {
        self.observed_concurrency = observed;
        self
    }

    /// Attach runtime ISL/OSL validation block.
    pub fn with_isl_osl(mut self, isl_osl: Option<IslOslValidation>) -> Self {
        self.isl_osl = isl_osl;
        self
    }

    /// Stamp TTFT approximation count and optional warning text.
    pub fn with_ttft_audit(mut self, approx_count: usize, warning: Option<String>) -> Self {
        self.ttft_approx_count = approx_count;
        self.ttft_warning = warning;
        self
    }

    /// Apply declared hourly price and derive `cost_per_million_output_tokens`.
    pub fn with_price(mut self, price: Option<(f64, &'static str)>) -> Self {
        match price {
            Some((price_per_hour, provenance)) => {
                self.price_per_hour = Some(price_per_hour);
                self.price_provenance = Some(provenance);
                self.cost_per_million_output_tokens = self
                    .completion_tokens_per_second
                    .and_then(|rate| cost_per_million_output_tokens(price_per_hour, rate));
            }
            None => {
                self.price_per_hour = None;
                self.price_provenance = None;
                self.cost_per_million_output_tokens = None;
            }
        }
        self
    }

    /// Print human-readable stats from this summary (Hyndman–Fan type 7).
    pub fn print_console(&self) {
        print_run_summary(self);
    }
}

fn fmt_opt(value: Option<f64>, precision: usize) -> String {
    match value {
        Some(v) => format!("{v:.precision$}"),
        None => "n/a".into(),
    }
}

fn print_dist(label: &str, dist: &DistSummary) {
    print_dist_unit(label, dist, "s");
}

fn print_dist_unit(label: &str, dist: &DistSummary, unit: &str) {
    if dist.n == 0 {
        println!("  {label}: n=0");
        return;
    }
    let mut flags = Vec::new();
    if dist.p90_unreliable {
        flags.push("p90*");
    }
    if dist.p95_unreliable {
        flags.push("p95*");
    }
    if dist.p99_unreliable {
        flags.push("p99*");
    }
    let flag = if flags.is_empty() {
        String::new()
    } else {
        format!(" [{}]", flags.join(","))
    };
    println!(
        "  {label}: n={} avg={}{unit} p50={}{unit} p90={}{unit} p95={}{unit} p99={}{unit}{flag}",
        dist.n,
        fmt_opt(dist.avg, 3),
        fmt_opt(dist.p50, 3),
        fmt_opt(dist.p90, 3),
        fmt_opt(dist.p95, 3),
        fmt_opt(dist.p99, 3),
    );
}

/// Render a [`RunSummary`] to stdout using DistSummary percentiles only.
pub fn print_run_summary(summary: &RunSummary) {
    println!("\n=== Run summary ({}) ===", summary.schema_version);
    if summary.partial {
        println!("  (partial - interrupted)");
    }
    println!(
        "  Attempted: {}  Successes: {}  Errors: {}  Error rate: {:.3}",
        summary.attempted, summary.successes, summary.errors, summary.error_rate
    );
    if !summary.errors_by_type.is_empty() {
        println!("  Errors by type: {:?}", summary.errors_by_type);
    }
    println!(
        "  Window: {:.3}s  Requests/sec: {:.3}",
        summary.window_seconds, summary.requests_per_second
    );
    match summary.completion_tokens_per_second {
        Some(rate) => println!(
            "  Completion tokens/sec: {:.3} ({})",
            rate,
            summary.completion_tokens_source.unwrap_or("unknown")
        ),
        None => {
            // Prefer modality metrics over a misleading n/a token line for ASR/imagegen.
            // Callers that only have token work still see the usage_missing note.
            if summary.ttft_s.n == 0 && summary.usage_missing_count == 0 {
                // Non-token modality: omit completion-token line (N-07).
            } else {
                println!(
                    "  Completion tokens/sec: n/a (usage_missing_count={})",
                    summary.usage_missing_count
                );
            }
        }
    }
    if let (Some(prompt), Some(rate)) =
        (summary.prompt_tokens_total, summary.input_tokens_per_second)
    {
        println!("  Prompt tokens: {prompt}  Input tokens/sec: {rate:.3}");
    }
    if let Some(completion) = summary.completion_tokens_total {
        println!("  Completion tokens: {completion}");
    }
    if let Some(rate) = summary.total_tokens_per_second {
        println!("  Total tokens/sec: {rate:.3}");
    }
    print_dist("Latency", &summary.latency_s);
    print_dist(
        "CO-corrected latency",
        &summary.coordinated_omission_latency_s,
    );
    print_dist("TTFT", &summary.ttft_s);
    if summary.ttft_approx_count > 0 {
        println!(
            "  TTFT approximated from first-byte: {}",
            summary.ttft_approx_count
        );
    }
    if let Some(warning) = &summary.ttft_warning {
        println!("  TTFT warning: {warning}");
    }
    print_dist("TPOT", &summary.tpot_s);
    print_dist("ITL", &summary.itl_s);
    if summary.connect_s.n > 0 {
        print_dist("Connect", &summary.connect_s);
    }
    if summary.prefill_s.n > 0 {
        print_dist("Prefill (proxy)", &summary.prefill_s);
    }
    if summary.decode_s.n > 0 {
        print_dist("Decode", &summary.decode_s);
    }
    if summary.decode_tok_s.n > 0 {
        println!(
            "  Decode tok/s: n={} avg={} p50={}",
            summary.decode_tok_s.n,
            fmt_opt(summary.decode_tok_s.avg, 3),
            fmt_opt(summary.decode_tok_s.p50, 3),
        );
    }
    if summary.time_to_second_token_s.n > 0 {
        print_dist("Time to second token", &summary.time_to_second_token_s);
    }
    if summary.prefill_tps_per_user.n > 0 {
        print_dist_unit("Prefill tok/s per user", &summary.prefill_tps_per_user, "");
    }
    if summary.user_tps.n > 0 {
        print_dist_unit("User tok/s", &summary.user_tps, "");
    }
    if summary.first_byte_s.n > 0 {
        print_dist("First byte", &summary.first_byte_s);
    }
    if summary.queue_delay_s.n > 0 {
        print_dist("Queue delay", &summary.queue_delay_s);
    }
    if summary.first_reasoning_s.n > 0 {
        print_dist("First reasoning", &summary.first_reasoning_s);
    }
    if summary.isl_tokens.n > 0 {
        print_dist_unit(
            &format!(
                "ISL tokens ({})",
                summary.isl_tokens_source.unwrap_or("unknown")
            ),
            &summary.isl_tokens,
            "",
        );
    }
    if summary.osl_tokens.n > 0 {
        print_dist_unit(
            &format!(
                "OSL tokens ({})",
                summary.osl_tokens_source.unwrap_or("unknown")
            ),
            &summary.osl_tokens,
            "",
        );
    }
    if summary.reasoning_tokens.n > 0 {
        print_dist_unit("Reasoning tokens", &summary.reasoning_tokens, "");
        println!(
            "  Reasoning tokens total: {}",
            summary.reasoning_tokens_total.unwrap_or(0)
        );
    }
    if summary.visible_completion_tokens.n > 0 {
        print_dist_unit(
            "Visible completion tokens",
            &summary.visible_completion_tokens,
            "",
        );
    }
    if let Some(obs) = &summary.observed_concurrency {
        println!(
            "  Observed concurrency: mean={} p50={} max={} cap={} engagement={}",
            fmt_opt(obs.in_flight_mean, 2),
            fmt_opt(obs.in_flight_p50, 2),
            fmt_opt(obs.in_flight_max, 2),
            obs.cap,
            fmt_opt(obs.cap_engagement_fraction, 3),
        );
    }
    if let Some(v) = &summary.isl_osl {
        println!(
            "  ISL/OSL: isl_mean={} osl_mean={} isl_mismatch={} osl_mismatch={} (targets isl={:?} osl={:?})",
            fmt_opt(v.isl_mean, 1),
            fmt_opt(v.osl_mean, 1),
            v.isl_mismatch_count,
            v.osl_mismatch_count,
            v.isl_target,
            v.osl_target,
        );
    }
    println!(
        "  Goodput: {:.3} req/s ({}/{} attempted; thresholds={:?})",
        summary.goodput.requests_per_second,
        summary.goodput.count,
        summary.attempted,
        summary.goodput.thresholds_s
    );
    if summary.pooled_mixture {
        for (name, ep) in &summary.per_endpoint {
            println!(
                "\n=== Endpoint: {name} (attempted={}, ok={}, err={}) ===",
                ep.attempted, ep.successes, ep.errors
            );
            print_dist("Latency", &ep.latency_s);
            print_dist("TTFT", &ep.ttft_s);
            print_dist("TPOT", &ep.tpot_s);
        }
    }
}

/// Run-level token sum for one usage field (prompt or completion).
struct TokenSum {
    usage_missing_count: usize,
    sum: u64,
    /// `None` when the sum is not reportable (unfilled gap or no token work).
    source: Option<&'static str>,
}

impl TokenSum {
    fn total(&self) -> Option<u64> {
        self.source.map(|_| self.sum)
    }
}

/// Sum one token field over successes. Server `usage` wins; the tokenizer
/// fills `usage_missing` rows. Any unfilled gap makes the sum unreportable.
fn token_sum_accounting(
    successes: &[&RequestRecord],
    pick: impl Fn(&RequestRecord) -> (u64, Option<u64>),
) -> TokenSum {
    let mut usage_missing_count = 0usize;
    let mut sum = 0u64;
    let mut used_tokenizer_fallback = false;
    let mut any_unfilled_gap = false;
    for record in successes {
        let (server, tokenized) = pick(record);
        if record.usage_missing {
            usage_missing_count += 1;
            if let Some(tokens) = tokenized {
                sum = sum.saturating_add(tokens);
                used_tokenizer_fallback = true;
            } else {
                any_unfilled_gap = true;
            }
        } else {
            sum = sum.saturating_add(server);
        }
    }
    let source = if any_unfilled_gap || successes.is_empty() {
        None
    } else if used_tokenizer_fallback {
        Some("tokenizer_fallback")
    } else if sum > 0 || usage_missing_count > 0 {
        Some("server_usage")
    } else {
        // Non-token modalities (ASR/imagegen) report 0 tokens without a
        // usage gap; do not claim server_usage token throughput (N-07).
        None
    };
    TokenSum {
        usage_missing_count,
        sum,
        source,
    }
}

impl CrossRunSummary {
    pub fn from_runs(runs: &[RunSummary], seed: u64) -> Self {
        let request_rates: Vec<_> = runs.iter().map(|r| r.requests_per_second).collect();
        let token_rates: Vec<_> = runs
            .iter()
            .filter_map(|r| r.completion_tokens_per_second)
            .collect();
        Self {
            runs: runs.len(),
            requests_per_second: DistSummary::from_values(&request_rates),
            requests_per_second_ci95: bootstrap_mean_ci(&request_rates, 0.95, 10_000, seed),
            completion_tokens_per_second: DistSummary::from_values(&token_rates),
            completion_tokens_per_second_ci95: bootstrap_mean_ci(
                &token_rates,
                0.95,
                10_000,
                seed.wrapping_add(1),
            ),
        }
    }
}

fn meets_slos(record: &&RequestRecord, slos: &SloConfig) -> bool {
    slos.e2e_s
        .is_none_or(|limit| record.corrected_latency_s() <= limit)
        && slos
            .ttft_s
            .is_none_or(|limit| record.ttft_s.is_some_and(|value| value <= limit))
        && slos
            .tpot_s
            .is_none_or(|limit| record.tpot_s().is_some_and(|value| value <= limit))
        && slos
            .user_tps
            .is_none_or(|min| record.user_tps().is_some_and(|value| value >= min))
}

fn endpoint_summaries(records: &[&RequestRecord]) -> BTreeMap<String, EndpointSummary> {
    let mut grouped: BTreeMap<String, Vec<&RequestRecord>> = BTreeMap::new();
    for record in records {
        grouped
            .entry(record.endpoint.clone())
            .or_default()
            .push(record);
    }
    grouped
        .into_iter()
        .map(|(name, rows)| {
            let success: Vec<_> = rows.iter().copied().filter(|r| r.is_success()).collect();
            // Same estimator as the pooled block (service latency). Coordinated-omission
            // correction lives only on coordinated_omission_latency_s.
            let lat: Vec<_> = success.iter().map(|r| r.latency_s).collect();
            let ttft: Vec<_> = success.iter().filter_map(|r| r.ttft_s).collect();
            let tpot: Vec<_> = success.iter().filter_map(|r| r.tpot_s()).collect();
            let itl: Vec<_> = success
                .iter()
                .flat_map(|r| r.itl_s.iter().copied())
                .collect();
            let recorded = RecordedFieldDists::from_successes(&success);
            (
                name,
                EndpointSummary {
                    attempted: rows.len(),
                    successes: success.len(),
                    errors: rows.len() - success.len(),
                    latency_s: DistSummary::from_values(&lat),
                    ttft_s: DistSummary::from_values(&ttft),
                    tpot_s: DistSummary::from_values(&tpot),
                    itl_s: DistSummary::from_values(&itl),
                    first_byte_s: recorded.first_byte_s,
                    queue_delay_s: recorded.queue_delay_s,
                    first_reasoning_s: recorded.first_reasoning_s,
                    isl_tokens: recorded.isl_tokens,
                    isl_tokens_source: recorded.isl_tokens_source,
                    osl_tokens: recorded.osl_tokens,
                    osl_tokens_source: recorded.osl_tokens_source,
                    reasoning_tokens: recorded.reasoning_tokens,
                    visible_completion_tokens: recorded.visible_completion_tokens,
                },
            )
        })
        .collect()
}

fn throughput_bins(records: &[&RequestRecord], window: f64, bin_seconds: f64) -> Vec<f64> {
    if records.is_empty() || !bin_seconds.is_finite() || bin_seconds <= 0.0 {
        return Vec::new();
    }
    let count = (window / bin_seconds).ceil().max(1.0) as usize;
    let mut bins = vec![0usize; count];
    let any_open_loop = records.iter().any(|r| r.scheduled_offset_s.is_some());
    if any_open_loop {
        for record in records {
            if let Some(offset) = record.scheduled_offset_s {
                let index = (offset / bin_seconds).floor() as usize;
                if let Some(bin) = bins.get_mut(index) {
                    *bin += 1;
                }
            }
        }
    } else {
        // Closed loop: bin by monotonic send offset from the first measured send.
        let min_send = records
            .iter()
            .map(|r| crate::runner::send_offset_seconds(r))
            .fold(f64::INFINITY, f64::min);
        if min_send.is_finite() {
            for record in records {
                let offset = crate::runner::send_offset_seconds(record) - min_send;
                let index = (offset / bin_seconds).floor() as usize;
                if let Some(bin) = bins.get_mut(index) {
                    *bin += 1;
                }
            }
        }
    }
    if bins.iter().all(|count| *count == 0) {
        return vec![records.len() as f64 / window.max(f64::EPSILON)];
    }
    // Normalize each bin by its actual width: trailing (and sole short-window)
    // bins use the remainder of `window`, not the full `bin_seconds` (N-06).
    bins.into_iter()
        .enumerate()
        .map(|(index, count)| {
            let start = index as f64 * bin_seconds;
            let width = (window - start).clamp(f64::EPSILON, bin_seconds);
            count as f64 / width
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args_common::{EffectiveCommonArgs, LoadBalancer};
    use crate::error::RequestError;
    use crate::record::Phase;
    use chrono::Utc;
    use std::time::Duration;

    fn with_times(started: chrono::DateTime<Utc>, latency: Duration) -> chrono::DateTime<Utc> {
        started + chrono::Duration::from_std(latency).unwrap()
    }

    fn ok(seq: u64, lat_ms: u64, ttft_ms: u64, tokens: u64, itl_ms: &[u64]) -> RequestRecord {
        let started = Utc::now();
        let latency = Duration::from_millis(lat_ms);
        RequestRecord::success(
            seq,
            Phase::Measure,
            "ep".into(),
            started,
            with_times(started, latency),
            latency,
            Some(Duration::from_millis(ttft_ms)),
            None,
            itl_ms.iter().map(|m| Duration::from_millis(*m)).collect(),
            18,
            tokens,
            18 + tokens,
        )
    }

    fn sample_common() -> EffectiveCommonArgs {
        EffectiveCommonArgs {
            scenario: Some("unit-test".into()),
            seed: 7,
            warmup_requests: 0,
            request_rate: None,
            arrival: "constant".into(),
            max_concurrency: None,
            load_balancer: LoadBalancer::RoundRobin,
            ignore_eos: false,
            min_tokens: None,
            extra_body_json: None,
            system_prompt: None,
            unique_prompts: false,
            tokenizer: None,
            slos: vec![],
            throughput_bin_seconds: 10.0,
            insecure: false,
            ca_cert: None,
            fail_on_error: false,
            price_per_hour: None,
            isl_target: None,
            osl_target: None,
            isl_tolerance: 0.0,
            osl_tolerance: 0.0,
            prompt_mix_report: None,
            fail_on_osl_mismatch: false,
            sut: None,
            require_sut: false,
            redact_hostname: false,
        }
    }

    #[test]
    fn excludes_no_output_token_from_ttft() {
        let mut recs = vec![ok(0, 500, 120, 20, &[20; 19])];
        let started = Utc::now();
        let latency = Duration::from_millis(300);
        recs.push(RequestRecord::failed(
            1,
            Phase::Measure,
            "ep".into(),
            started,
            with_times(started, latency),
            latency,
            RequestError::NoOutputToken,
        ));
        let s = RunSummary::from_records(&recs, 1.0, false);
        assert_eq!(s.ttft_s.n, 1);
        assert_eq!(s.errors_by_type.get("no_output_token").copied(), Some(1));
        assert!((s.error_rate - 0.5).abs() < 1e-9);
    }

    #[test]
    fn error_rate_uses_attempted() {
        let recs: Vec<_> = (0..120)
            .map(|i| {
                if i < 10 {
                    let started = Utc::now();
                    let latency = Duration::from_millis(10);
                    RequestRecord::failed(
                        i,
                        Phase::Measure,
                        "ep".into(),
                        started,
                        with_times(started, latency),
                        latency,
                        RequestError::RateLimit,
                    )
                } else {
                    ok(i, 100, 20, 8, &[])
                }
            })
            .collect();
        let s = RunSummary::from_records(&recs, 1.0, false);
        assert!((s.error_rate - 10.0 / 120.0).abs() < 1e-9);
    }

    #[test]
    fn per_endpoint_and_goodput_are_independent() {
        let mut a = ok(0, 100, 20, 8, &[10]);
        a.endpoint = "fast".into();
        let mut b = ok(1, 500, 200, 8, &[50]);
        b.endpoint = "slow".into();
        let records = [a, b];
        let slo = SloConfig::parse(&["e2e=250ms".into()]).unwrap();
        let summary = RunSummary::from_records_with_options(&records, 1.0, false, &slo, 0.5);
        assert!(summary.pooled_mixture);
        assert_eq!(summary.per_endpoint.len(), 2);
        assert_eq!(summary.goodput.count, 1);
    }

    #[test]
    fn cross_run_summary_reports_dispersion_and_seeded_ci() {
        let records = [ok(0, 100, 20, 8, &[])];
        let a = RunSummary::from_records(&records, 1.0, false);
        let b = RunSummary::from_records(&records, 0.5, false);
        let summary = CrossRunSummary::from_runs(&[a, b], 9);
        assert_eq!(summary.runs, 2);
        assert_eq!(summary.requests_per_second.n, 2);
        assert_eq!(summary.requests_per_second.avg, Some(1.5));
        assert!(summary.requests_per_second_ci95.low.is_some());
    }

    #[test]
    fn usage_missing_nulls_ctps_without_tokenizer() {
        let mut rec = ok(0, 100, 20, 0, &[]);
        rec.usage_missing = true;
        let summary = RunSummary::from_records(&[rec], 1.0, false);
        assert_eq!(summary.usage_missing_count, 1);
        assert!(summary.completion_tokens_per_second.is_none());
        assert!(summary.completion_tokens_source.is_none());
        let value = serde_json::to_value(&summary).unwrap();
        assert!(value["completion_tokens_per_second"].is_null());
    }

    #[test]
    fn usage_missing_uses_tokenizer_fallback() {
        let mut rec = ok(0, 100, 20, 0, &[]);
        rec.usage_missing = true;
        rec.tokenized_completion_tokens = Some(20);
        let summary = RunSummary::from_records(&[rec], 1.0, false);
        assert_eq!(summary.usage_missing_count, 1);
        assert_eq!(summary.completion_tokens_per_second, Some(20.0));
        assert_eq!(summary.completion_tokens_source, Some("tokenizer_fallback"));
    }

    #[test]
    fn server_usage_source_when_present() {
        let summary = RunSummary::from_records(&[ok(0, 100, 20, 8, &[])], 1.0, false);
        assert_eq!(summary.usage_missing_count, 0);
        assert_eq!(summary.completion_tokens_per_second, Some(8.0));
        assert_eq!(summary.completion_tokens_source, Some("server_usage"));
    }

    #[test]
    fn with_config_stamps_effective_run_config() {
        let summary = RunSummary::from_records(&[ok(0, 100, 20, 8, &[])], 1.0, false).with_config(
            EffectiveRunConfig {
                run_id: "run-1".into(),
                common: sample_common(),
                effective_max_concurrency: 8,
                effective_system_prompt: Some("You are a helpful assistant.".into()),
                body_template: serde_json::json!({"prompt": "{{prompt}}"}),
                unique_prompt_nonce_template: None,
                modality: BTreeMap::new(),
            },
        );
        let cfg = summary.config.expect("config stamped");
        assert_eq!(cfg.run_id, "run-1");
        assert_eq!(cfg.common.seed, 7);
    }

    #[test]
    fn trailing_partial_bin_uses_actual_width() {
        // Window 12.15 s, bin 10 s: first bin full width, trailing 2.15 s.
        // 247 requests in first 10 s + 52 in the last 2.15 s.
        let t0 = Utc::now();
        let mut records = Vec::new();
        for i in 0..247 {
            let offset = (i as f64) * (10.0 / 247.0);
            records.push(ok(i, 100, 20, 8, &[]).with_send_offset(Duration::from_secs_f64(offset)));
            records.last_mut().unwrap().started_at = t0;
        }
        for i in 0..52 {
            let offset = 10.0 + (i as f64) * (2.15 / 52.0);
            records.push(
                ok(247 + i, 100, 20, 8, &[]).with_send_offset(Duration::from_secs_f64(offset)),
            );
            records.last_mut().unwrap().started_at = t0;
        }
        let bins = throughput_bins(&records.iter().collect::<Vec<_>>(), 12.15, 10.0);
        assert_eq!(bins.len(), 2);
        assert!(
            (bins[0] - 24.7).abs() < 0.5,
            "full bin rps={}, expected ~24.7",
            bins[0]
        );
        assert!(
            (bins[1] - (52.0 / 2.15)).abs() < 1.0,
            "trailing bin rps={}, expected ~{}",
            bins[1],
            52.0 / 2.15
        );
    }

    #[test]
    fn short_window_bin_uses_window_not_full_bin_seconds() {
        let t0 = Utc::now();
        let records: Vec<_> = (0..10)
            .map(|i| {
                let mut r = ok(i, 50, 10, 4, &[]).with_send_offset(Duration::from_millis(i * 20));
                r.started_at = t0;
                r
            })
            .collect();
        // Window 0.2 s with bin_seconds 1.0 must not report 10.0 / 1.0 = 10 rps.
        let bins = throughput_bins(&records.iter().collect::<Vec<_>>(), 0.2, 1.0);
        assert_eq!(bins.len(), 1);
        assert!(
            (bins[0] - 50.0).abs() < 1e-6,
            "short-window rps={}, expected 50",
            bins[0]
        );
    }

    #[test]
    fn parses_user_tps_slo_and_filters_goodput() {
        let slo = SloConfig::parse(&["user_tps=25".into()]).unwrap();
        assert_eq!(slo.user_tps, Some(25.0));
        // 20 tokens in 0.5s => 40 tok/s (meets); 8 tokens in 0.5s => 16 tok/s (fails).
        let fast = ok(0, 500, 100, 20, &[20; 19]);
        let slow = ok(1, 500, 100, 8, &[50; 7]);
        let summary = RunSummary::from_records_with_options(&[fast, slow], 1.0, false, &slo, 10.0);
        assert_eq!(summary.goodput.count, 1);
        assert_eq!(summary.goodput.thresholds_s.get("user_tps"), Some(&25.0));
    }

    #[test]
    fn cost_per_million_from_price_and_token_rate() {
        let expected = 3.6 / (100.0 * 3600.0) * 1_000_000.0;
        assert!((cost_per_million_output_tokens(3.6, 100.0).unwrap() - expected).abs() < 1e-9);
        assert!(cost_per_million_output_tokens(3.6, 0.0).is_none());
        let records = [ok(0, 100, 20, 10, &[])];
        let summary = RunSummary::from_records(&records, 1.0, false).with_price(Some((3.6, "cli")));
        assert_eq!(summary.price_provenance, Some("cli"));
        assert!(summary.cost_per_million_output_tokens.is_some());
        let value = serde_json::to_value(&summary).unwrap();
        assert!(value["cost_per_million_output_tokens"].is_number());
        let null_summary = RunSummary::from_records(&records, 1.0, false).with_price(None);
        let null_value = serde_json::to_value(&null_summary).unwrap();
        assert!(null_value["cost_per_million_output_tokens"].is_null());
    }

    #[test]
    fn summary_includes_phase_and_concurrency_blocks() {
        let mut rec = ok(0, 500, 120, 20, &[20; 19]).with_connect(0.02);
        rec = rec.with_in_flight(3);
        let summary = RunSummary::from_records(&[rec], 1.0, false).with_observed_concurrency(Some(
            crate::concurrency::ObservedConcurrency {
                cap: 8,
                in_flight_mean: Some(3.0),
                in_flight_p50: Some(3.0),
                in_flight_max: Some(4.0),
                cap_engagement_fraction: Some(0.25),
                acquire_count: 4,
                wait_count: 1,
            },
        ));
        assert_eq!(summary.connect_s.n, 1);
        assert_eq!(summary.prefill_s.n, 1);
        assert_eq!(summary.decode_s.n, 1);
        assert!(summary.decode_tok_s.avg.unwrap() > 0.0);
        assert_eq!(summary.observed_concurrency.as_ref().unwrap().cap, 8);
        let isl = crate::isl_osl::validate_records(
            &[ok(0, 100, 20, 8, &[])],
            &crate::isl_osl::IslOslTargets {
                isl_target: Some(18.0),
                osl_target: Some(8.0),
                isl_tolerance: 0.0,
                osl_tolerance: 0.0,
                source: Some("cli"),
            },
        )
        .unwrap();
        assert_eq!(isl.osl_mismatch_count, 0);
        assert_eq!(isl.isl_mismatch_count, 0);
    }
    #[test]
    fn recorded_fields_present_on_streaming_chat_records() {
        let mut a = ok(0, 500, 120, 20, &[20; 19]).with_first_byte(Duration::from_millis(40));
        a.first_reasoning_s = Some(0.05);
        let mut b = ok(1, 600, 130, 30, &[20; 29]).with_first_byte(Duration::from_millis(50));
        b.first_reasoning_s = Some(0.07);
        b.endpoint = "other".into();
        let summary = RunSummary::from_records(&[a, b], 1.0, false);
        assert_eq!(summary.first_byte_s.n, 2);
        assert_eq!(summary.first_reasoning_s.n, 2);
        assert_eq!(summary.isl_tokens.n, 2);
        assert_eq!(summary.isl_tokens.avg, Some(18.0));
        assert_eq!(summary.osl_tokens.n, 2);
        assert_eq!(summary.osl_tokens.avg, Some(25.0));
        assert_eq!(summary.isl_tokens_source, Some("server_usage"));
        assert_eq!(summary.osl_tokens_source, Some("server_usage"));
        // Closed loop: no intended arrival, so queue delay is not applicable.
        assert_eq!(summary.queue_delay_s.n, 0);
        let ep = &summary.per_endpoint["other"];
        assert_eq!(ep.first_byte_s.n, 1);
        assert_eq!(ep.osl_tokens.avg, Some(30.0));
        assert_eq!(ep.osl_tokens_source, Some("server_usage"));
        let value = serde_json::to_value(&summary).unwrap();
        assert_eq!(value["first_byte_s"]["n"], 2);
        assert_eq!(value["per_endpoint"]["ep"]["isl_tokens"]["n"], 1);
    }

    #[test]
    fn queue_delay_only_from_scheduled_requests() {
        let open = ok(0, 100, 20, 8, &[])
            .with_schedule(Duration::from_millis(10), Duration::from_millis(3));
        let closed = ok(1, 100, 20, 8, &[]);
        let summary = RunSummary::from_records(&[open, closed], 1.0, false);
        assert_eq!(summary.queue_delay_s.n, 1);
        assert!((summary.queue_delay_s.avg.unwrap() - 0.003).abs() < 1e-12);
    }

    #[test]
    fn recorded_fields_never_fabricated_for_non_token_rows() {
        // ASR / imagegen shape: zero usage and no usage_missing flag.
        let rec = ok(0, 100, 20, 0, &[]);
        let mut rec = rec;
        rec.prompt_tokens = 0;
        rec.total_tokens = 0;
        rec.ttft_s = None;
        let summary = RunSummary::from_records(&[rec], 1.0, false);
        for dist in [
            &summary.first_byte_s,
            &summary.queue_delay_s,
            &summary.first_reasoning_s,
            &summary.isl_tokens,
            &summary.osl_tokens,
        ] {
            assert_eq!(dist.n, 0);
            assert!(dist.avg.is_none());
        }
        assert!(summary.isl_tokens_source.is_none());
        let value = serde_json::to_value(&summary).unwrap();
        assert!(value.get("osl_tokens_source").is_none());
    }

    #[test]
    fn reasoning_tokens_summarized_only_from_reporting_rows() {
        let a = ok(0, 500, 120, 20, &[20; 19]).with_reasoning_tokens(Some(6));
        let mut b = ok(1, 600, 130, 30, &[20; 29]).with_reasoning_tokens(Some(10));
        b.endpoint = "other".into();
        // Server reported no reasoning count: excluded, not counted as zero.
        let c = ok(2, 500, 120, 20, &[20; 19]);
        let summary = RunSummary::from_records(&[a, b, c], 1.0, false);
        assert_eq!(summary.reasoning_tokens.n, 2);
        assert_eq!(summary.reasoning_tokens.avg, Some(8.0));
        assert_eq!(summary.reasoning_tokens_total, Some(16));
        assert_eq!(summary.visible_completion_tokens.n, 2);
        assert_eq!(summary.visible_completion_tokens.avg, Some(17.0));
        assert_eq!(summary.visible_completion_tokens_total, Some(34));
        assert_eq!(summary.per_endpoint["other"].reasoning_tokens.n, 1);
        assert_eq!(
            summary.per_endpoint["other"].visible_completion_tokens.avg,
            Some(20.0)
        );
    }

    #[test]
    fn reasoning_total_saturates_instead_of_overflowing() {
        assert_eq!(total(&[u64::MAX, 5]), Some(u64::MAX));
        assert_eq!(total(&[]), None);
    }

    #[test]
    fn reasoning_tokens_absent_serialize_as_null() {
        let rec = ok(0, 500, 120, 20, &[20; 19]);
        let record_value = serde_json::to_value(&rec).unwrap();
        assert!(record_value["reasoning_tokens"].is_null());
        assert!(record_value["visible_completion_tokens"].is_null());
        assert!(record_value
            .as_object()
            .unwrap()
            .contains_key("reasoning_tokens"));
        let summary = RunSummary::from_records(&[rec], 1.0, false);
        assert_eq!(summary.reasoning_tokens.n, 0);
        let value = serde_json::to_value(&summary).unwrap();
        assert!(value["reasoning_tokens_total"].is_null());
        assert!(value["visible_completion_tokens_total"].is_null());
        assert_eq!(value["reasoning_tokens"]["n"], 0);
    }

    #[test]
    fn token_totals_and_rates_cross_check_per_request() {
        let mut records = vec![
            ok(0, 500, 100, 20, &[20; 19]),
            ok(1, 1000, 200, 40, &[50, 30]),
            // Unary: no TTFT and no ITL.
            RequestRecord::success(
                2,
                Phase::Measure,
                "ep".into(),
                Utc::now(),
                Utc::now(),
                Duration::from_millis(400),
                None,
                None,
                Vec::new(),
                30,
                10,
                40,
            ),
        ];
        // Errors and warmup never count.
        let mut warm = ok(3, 500, 100, 20, &[20]);
        warm.phase = Phase::Warmup;
        records.push(warm);
        let window = 4.0;
        let summary = RunSummary::from_records(&records, window, false);

        // Recompute every value from the per-request rows.
        let measured: Vec<&RequestRecord> = records
            .iter()
            .filter(|r| r.phase == Phase::Measure && r.is_success())
            .collect();
        let prompt: u64 = measured.iter().map(|r| r.prompt_tokens).sum();
        let completion: u64 = measured.iter().map(|r| r.completion_tokens).sum();
        assert_eq!(summary.prompt_tokens_total, Some(prompt));
        assert_eq!(summary.completion_tokens_total, Some(completion));
        assert_eq!(
            summary.input_tokens_per_second,
            Some(prompt as f64 / window)
        );
        assert_eq!(
            summary.total_tokens_per_second,
            Some((prompt + completion) as f64 / window)
        );
        assert_eq!(
            summary.completion_tokens_per_second,
            Some(completion as f64 / window)
        );

        let prefill: Vec<f64> = measured
            .iter()
            .filter_map(|r| Some(r.prompt_tokens as f64 / r.ttft_s?))
            .collect();
        let second: Vec<f64> = measured
            .iter()
            .filter_map(|r| Some(r.ttft_s? + r.itl_s.first()?))
            .collect();
        let user: Vec<f64> = measured
            .iter()
            .map(|r| r.completion_tokens as f64 / r.latency_s)
            .collect();
        for (dist, values) in [
            (&summary.prefill_tps_per_user, &prefill),
            (&summary.time_to_second_token_s, &second),
            (&summary.user_tps, &user),
        ] {
            let expected = DistSummary::from_values(values);
            assert_eq!(dist.n, expected.n);
            assert!((dist.avg.unwrap() - expected.avg.unwrap()).abs() < 1e-9);
            assert!((dist.p50.unwrap() - expected.p50.unwrap()).abs() < 1e-9);
        }
        assert_eq!(summary.prefill_tps_per_user.n, 2);
        assert!((summary.prefill_tps_per_user.p50.unwrap() - 135.0).abs() < 1e-9);
        assert_eq!(summary.time_to_second_token_s.n, 2);
        assert_eq!(summary.user_tps.n, 3);

        let value = serde_json::to_value(&summary).unwrap();
        for key in [
            "prompt_tokens_total",
            "completion_tokens_total",
            "input_tokens_per_second",
            "total_tokens_per_second",
            "prefill_tps_per_user",
            "time_to_second_token_s",
            "user_tps",
        ] {
            assert!(value.get(key).is_some(), "{key} missing");
        }
    }

    #[test]
    fn token_totals_null_without_token_work_or_with_unfilled_gap() {
        // ASR/imagegen-style rows: no usage, not flagged missing.
        let mut bare = ok(0, 500, 100, 0, &[]);
        bare.prompt_tokens = 0;
        bare.total_tokens = 0;
        let none = RunSummary::from_records(&[bare], 1.0, false);
        assert!(none.prompt_tokens_total.is_none());
        assert!(none.completion_tokens_total.is_none());
        assert!(none.input_tokens_per_second.is_none());
        assert!(none.total_tokens_per_second.is_none());
        assert_eq!(none.prefill_tps_per_user.n, 0);
        assert_eq!(none.user_tps.n, 0);
        let value = serde_json::to_value(&none).unwrap();
        assert!(value["prompt_tokens_total"].is_null());
        assert!(value["total_tokens_per_second"].is_null());

        // A usage gap with no tokenizer count makes totals unreportable.
        let server = ok(0, 500, 100, 8, &[]);
        let mut gap = ok(1, 500, 100, 0, &[]);
        gap.usage_missing = true;
        gap.prompt_tokens = 0;
        gap.total_tokens = 0;
        let unfilled = RunSummary::from_records(&[server.clone(), gap.clone()], 1.0, false);
        assert!(unfilled.prompt_tokens_total.is_none());
        assert!(unfilled.completion_tokens_total.is_none());
        assert!(unfilled.total_tokens_per_second.is_none());

        // Tokenizer-filled gap counts, matching isl_tokens / osl_tokens.
        gap.tokenized_prompt_tokens = Some(17);
        gap.tokenized_completion_tokens = Some(12);
        let filled = RunSummary::from_records(&[server, gap], 2.0, false);
        assert_eq!(filled.prompt_tokens_total, Some(18 + 17));
        assert_eq!(filled.completion_tokens_total, Some(8 + 12));
        assert_eq!(filled.total_tokens_per_second, Some(55.0 / 2.0));
        assert_eq!(filled.prefill_tps_per_user.n, 2);

        // First-byte approximated TTFT is excluded from the prefill rate only.
        let mut approx = ok(0, 500, 100, 20, &[]);
        approx.ttft_source = Some(crate::measurement::TtftSource::FirstByteApprox);
        let summary = RunSummary::from_records(&[approx], 1.0, false);
        assert_eq!(summary.prefill_tps_per_user.n, 0);
        assert_eq!(summary.ttft_s.n, 1);
        assert_eq!(summary.prompt_tokens_total, Some(18));
    }

    #[test]
    fn token_dists_label_tokenizer_fallback_and_mixed() {
        let server = ok(0, 100, 20, 8, &[]);
        let mut gap = ok(1, 100, 20, 0, &[]);
        gap.usage_missing = true;
        gap.prompt_tokens = 0;
        gap.total_tokens = 0;
        gap.tokenized_prompt_tokens = Some(17);
        gap.tokenized_completion_tokens = Some(12);
        let mut unfilled = gap.clone();
        unfilled.tokenized_prompt_tokens = None;
        unfilled.tokenized_completion_tokens = None;
        let only_gap = RunSummary::from_records(&[gap.clone(), unfilled.clone()], 1.0, false);
        assert_eq!(only_gap.isl_tokens.n, 1);
        assert_eq!(only_gap.isl_tokens.avg, Some(17.0));
        assert_eq!(only_gap.osl_tokens_source, Some("tokenizer_fallback"));
        let mixed = RunSummary::from_records(&[server, gap, unfilled], 1.0, false);
        assert_eq!(mixed.osl_tokens.n, 2);
        assert_eq!(mixed.osl_tokens.avg, Some(10.0));
        assert_eq!(mixed.osl_tokens_source, Some("mixed"));
        assert_eq!(mixed.isl_tokens_source, Some("mixed"));
    }
}
