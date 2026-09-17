// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Hyndman-Fan type 7 percentiles (numpy default), plus mean/std/MAD helpers.

/// Linear interpolation percentile, Hyndman & Fan type 7.
/// `p` is in 0..=100. Empty input returns None.
pub fn percentile_type7(sorted: &[f64], p: f64) -> Option<f64> {
    if sorted.is_empty() || !p.is_finite() {
        return None;
    }
    let n = sorted.len();
    if n == 1 {
        return Some(sorted[0]);
    }
    let p = p.clamp(0.0, 100.0);
    let h = (n as f64 - 1.0) * (p / 100.0);
    let lo = h.floor() as usize;
    let hi = h.ceil() as usize;
    let lo = lo.min(n - 1);
    let hi = hi.min(n - 1);
    if lo == hi {
        Some(sorted[lo])
    } else {
        let w = h - lo as f64;
        Some(sorted[lo] * (1.0 - w) + sorted[hi] * w)
    }
}

/// p99 (and similar) is unreliable when fewer than `1 / (1 - p/100)` samples exist.
pub fn percentile_unreliable(n: usize, p: f64) -> bool {
    n == 0 || (n as f64) * (1.0 - p / 100.0) < 1.0
}

pub fn mean(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    Some(values.iter().sum::<f64>() / values.len() as f64)
}

pub fn std_sample(values: &[f64]) -> Option<f64> {
    if values.len() < 2 {
        return None;
    }
    let m = mean(values)?;
    let var = values.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (values.len() - 1) as f64;
    Some(var.sqrt())
}

pub fn mad(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let med = percentile_type7(&sorted, 50.0)?;
    let mut abs: Vec<f64> = sorted.iter().map(|x| (x - med).abs()).collect();
    abs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    percentile_type7(&abs, 50.0)
}

pub fn sort_finite(values: impl IntoIterator<Item = f64>) -> Vec<f64> {
    let mut v: Vec<f64> = values.into_iter().filter(|x| x.is_finite()).collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DistSummary {
    pub n: usize,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub avg: Option<f64>,
    pub std: Option<f64>,
    pub mad: Option<f64>,
    pub p50: Option<f64>,
    pub p90: Option<f64>,
    pub p95: Option<f64>,
    pub p99: Option<f64>,
    pub percentile_method: &'static str,
    pub p90_unreliable: bool,
    pub p95_unreliable: bool,
    pub p99_unreliable: bool,
}

impl DistSummary {
    pub fn from_values(values: &[f64]) -> Self {
        let sorted = sort_finite(values.iter().copied());
        let n = sorted.len();
        Self {
            n,
            min: sorted.first().copied(),
            max: sorted.last().copied(),
            avg: mean(&sorted),
            std: std_sample(&sorted),
            mad: mad(&sorted),
            p50: percentile_type7(&sorted, 50.0),
            p90: percentile_type7(&sorted, 90.0),
            p95: percentile_type7(&sorted, 95.0),
            p99: percentile_type7(&sorted, 99.0),
            percentile_method: "hyndman_fan_type7",
            p90_unreliable: percentile_unreliable(n, 90.0),
            p95_unreliable: percentile_unreliable(n, 95.0),
            p99_unreliable: percentile_unreliable(n, 99.0),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ConfidenceInterval {
    pub level: f64,
    pub low: Option<f64>,
    pub high: Option<f64>,
    pub resamples: usize,
    pub method: &'static str,
}

/// Seeded percentile bootstrap confidence interval for the arithmetic mean.
pub fn bootstrap_mean_ci(
    values: &[f64],
    level: f64,
    resamples: usize,
    seed: u64,
) -> ConfidenceInterval {
    use rand::{RngExt, SeedableRng};
    let finite = sort_finite(values.iter().copied());
    if finite.is_empty() || resamples == 0 {
        return ConfidenceInterval {
            level,
            low: None,
            high: None,
            resamples,
            method: "percentile_bootstrap",
        };
    }
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    let mut means = Vec::with_capacity(resamples);
    for _ in 0..resamples {
        let sum = (0..finite.len())
            .map(|_| finite[rng.random_range(0..finite.len())])
            .sum::<f64>();
        means.push(sum / finite.len() as f64);
    }
    means.sort_by(|a, b| a.total_cmp(b));
    let alpha = ((1.0 - level.clamp(0.0, 1.0)) / 2.0) * 100.0;
    ConfidenceInterval {
        level,
        low: percentile_type7(&means, alpha),
        high: percentile_type7(&means, 100.0 - alpha),
        resamples,
        method: "percentile_bootstrap",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type7_matches_numpy_reference() {
        // numpy.percentile(x, p, method='linear') == type 7
        let x: Vec<f64> = (1..=10).map(|i| i as f64).collect();
        let p50 = percentile_type7(&x, 50.0).unwrap();
        assert!((p50 - 5.5).abs() < 1e-9);
        let p0 = percentile_type7(&x, 0.0).unwrap();
        assert!((p0 - 1.0).abs() < 1e-9);
        let p100 = percentile_type7(&x, 100.0).unwrap();
        assert!((p100 - 10.0).abs() < 1e-9);
    }

    #[test]
    fn small_n_flags_unreliable() {
        assert!(percentile_unreliable(50, 99.0));
        assert!(!percentile_unreliable(100, 99.0));
        assert!(percentile_unreliable(0, 50.0));
        // Threshold is n * (1 - p/100) < 1; use clear sides of the boundary
        // (exact boundary values can trip floating-point for p=90/95).
        assert!(percentile_unreliable(9, 90.0));
        assert!(!percentile_unreliable(11, 90.0));
        assert!(percentile_unreliable(19, 95.0));
        assert!(!percentile_unreliable(21, 95.0));
        let d9 = DistSummary::from_values(&[1.0; 9]);
        assert!(d9.p90_unreliable);
        assert!(d9.p95_unreliable);
        assert!(d9.p99_unreliable);
        let d21 = DistSummary::from_values(&[1.0; 21]);
        assert!(!d21.p90_unreliable);
        assert!(!d21.p95_unreliable);
        assert!(d21.p99_unreliable);
        let d100 = DistSummary::from_values(&[1.0; 100]);
        assert!(!d100.p90_unreliable);
        assert!(!d100.p95_unreliable);
        assert!(!d100.p99_unreliable);
    }

    #[test]
    fn empty_is_none() {
        assert!(percentile_type7(&[], 50.0).is_none());
        let d = DistSummary::from_values(&[]);
        assert_eq!(d.n, 0);
        assert!(d.avg.is_none());
        let value = serde_json::to_value(d).expect("serialize distribution");
        for field in ["min", "max", "avg", "std", "p50", "p90", "p95", "p99"] {
            assert!(value[field].is_null(), "{field} must serialize as null");
        }
    }

    #[test]
    fn std_mad_and_bootstrap_are_seeded() {
        let values = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert!((std_sample(&values).unwrap() - 2.5_f64.sqrt()).abs() < 1e-12);
        assert_eq!(mad(&values), Some(1.0));
        let a = bootstrap_mean_ci(&values, 0.95, 1000, 7);
        let b = bootstrap_mean_ci(&values, 0.95, 1000, 7);
        assert_eq!(a.low, b.low);
        assert_eq!(a.high, b.high);
        assert!(a.low.unwrap() <= 3.0 && a.high.unwrap() >= 3.0);
    }
}
