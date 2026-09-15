// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::record::{Phase, RequestRecord, SCHEMA_VERSION_SUMMARY};
use crate::stats::{bootstrap_mean_ci, ConfidenceInterval, DistSummary};
use serde::Serialize;
use std::collections::BTreeMap;

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
    pub completion_tokens_per_second: f64,
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
        let completion_tokens: u64 = successes.iter().map(|r| r.completion_tokens).sum();
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
            completion_tokens_per_second: completion_tokens as f64 / window,
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
        }
    }
}

impl CrossRunSummary {
    pub fn from_runs(runs: &[RunSummary], seed: u64) -> Self {
        let request_rates: Vec<_> = runs.iter().map(|r| r.requests_per_second).collect();
        let token_rates: Vec<_> = runs
            .iter()
            .map(|r| r.completion_tokens_per_second)
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
        // Closed loop: bin by actual send offset from the first measured send.
        let min_send = records
            .iter()
            .map(|r| datetime_to_unix_secs(r.started_at))
            .fold(f64::INFINITY, f64::min);
        if min_send.is_finite() {
            for record in records {
                let offset = datetime_to_unix_secs(record.started_at) - min_send;
                let index = (offset / bin_seconds).floor() as usize;
                if let Some(bin) = bins.get_mut(index) {
                    *bin += 1;
                }
            }
        }
    }
    if bins.iter().all(|count| *count == 0) {
        return vec![records.len() as f64 / window];
    }
    bins.into_iter()
        .map(|count| count as f64 / bin_seconds)
        .collect()
}

fn datetime_to_unix_secs(ts: chrono::DateTime<chrono::Utc>) -> f64 {
    ts.timestamp() as f64 + f64::from(ts.timestamp_subsec_nanos()) / 1e9
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
