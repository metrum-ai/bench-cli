// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::args_common::EffectiveCommonArgs;
use crate::record::{Phase, RequestRecord, SCHEMA_VERSION_SUMMARY};
use crate::stats::{bootstrap_mean_ci, ConfidenceInterval, DistSummary};
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
    pub latency_s: DistSummary,
    pub coordinated_omission_latency_s: DistSummary,
    pub ttft_s: DistSummary,
    pub tpot_s: DistSummary,
    pub itl_s: DistSummary,
    pub throughput_bins_rps: DistSummary,
    pub goodput: GoodputSummary,
    pub pooled_mixture: bool,
    pub per_endpoint: BTreeMap<String, EndpointSummary>,
    pub environment: serde_json::Value,
    pub partial: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<EffectiveRunConfig>,
}

#[derive(Debug, Clone, Default)]
pub struct SloConfig {
    pub ttft_s: Option<f64>,
    pub tpot_s: Option<f64>,
    pub e2e_s: Option<f64>,
}

impl SloConfig {
    pub fn parse(values: &[String]) -> anyhow::Result<Self> {
        let mut config = Self::default();
        for value in values {
            let (name, raw) = value
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("SLO must be METRIC=SECONDS: {value}"))?;
            let seconds = parse_duration_seconds(raw)?;
            match name {
                "ttft" => config.ttft_s = Some(seconds),
                "tpot" => config.tpot_s = Some(seconds),
                "e2e" | "latency" => config.e2e_s = Some(seconds),
                _ => anyhow::bail!("unknown SLO metric '{name}'"),
            }
        }
        Ok(config)
    }
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
    /// all records — attempted is the slice length).
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
        let (usage_missing_count, completion_tokens, completion_tokens_source, ctps_valid) =
            token_throughput_accounting(&successes);
        let window = if window_seconds > 0.0 {
            window_seconds
        } else {
            1e-9
        };
        let good: Vec<_> = successes
            .iter()
            .filter(|record| meets_slos(record, slos))
            .collect();
        let thresholds_s = [
            ("ttft", slos.ttft_s),
            ("tpot", slos.tpot_s),
            ("e2e", slos.e2e_s),
        ]
        .into_iter()
        .filter_map(|(name, value)| value.map(|v| (name.to_string(), v)))
        .collect();
        let per_endpoint = endpoint_summaries(&pool);
        let bins = throughput_bins(&successes, window, bin_seconds);
        let completion_tokens_per_second = if ctps_valid {
            Some(completion_tokens as f64 / window)
        } else {
            None
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
            completion_tokens_source: if ctps_valid {
                completion_tokens_source
            } else {
                None
            },
            latency_s: DistSummary::from_values(&lat),
            coordinated_omission_latency_s: DistSummary::from_values(&corrected_lat),
            ttft_s: DistSummary::from_values(&ttft),
            tpot_s: DistSummary::from_values(&tpot),
            itl_s: DistSummary::from_values(&itl),
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
            environment: crate::environment::collect(None, None),
            partial,
            config: None,
        }
    }

    /// Stamp effective run configuration after summary construction.
    pub fn with_config(mut self, config: EffectiveRunConfig) -> Self {
        self.config = Some(config);
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
        "  {label}: n={} avg={}s p50={}s p90={}s p95={}s p99={}s{flag}",
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
        println!("  (partial — interrupted)");
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
    print_dist("Latency", &summary.latency_s);
    print_dist(
        "CO-corrected latency",
        &summary.coordinated_omission_latency_s,
    );
    print_dist("TTFT", &summary.ttft_s);
    print_dist("TPOT", &summary.tpot_s);
    print_dist("ITL", &summary.itl_s);
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

/// Returns `(usage_missing_count, token_sum, source, ctps_valid)`.
fn token_throughput_accounting(
    successes: &[&RequestRecord],
) -> (usize, u64, Option<&'static str>, bool) {
    let mut usage_missing_count = 0usize;
    let mut completion_tokens = 0u64;
    let mut used_tokenizer_fallback = false;
    let mut any_unfilled_gap = false;
    for record in successes {
        if record.usage_missing {
            usage_missing_count += 1;
            if let Some(tokens) = record.tokenized_completion_tokens {
                completion_tokens += tokens;
                used_tokenizer_fallback = true;
            } else {
                any_unfilled_gap = true;
            }
        } else {
            completion_tokens += record.completion_tokens;
        }
    }
    if any_unfilled_gap {
        return (usage_missing_count, 0, None, false);
    }
    let source = if successes.is_empty() {
        None
    } else if used_tokenizer_fallback {
        Some("tokenizer_fallback")
    } else if completion_tokens > 0 || usage_missing_count > 0 {
        Some("server_usage")
    } else {
        // Non-token modalities (ASR/imagegen) report 0 completion tokens without
        // a usage gap; do not claim server_usage token throughput (N-07).
        None
    };
    let ctps_valid = source.is_some();
    (usage_missing_count, completion_tokens, source, ctps_valid)
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
}
