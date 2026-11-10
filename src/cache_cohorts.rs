// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Measured request cohorts from server-reported cached-token evidence. Never
//! attribute aggregate external counters or a physical tier to a request.

use crate::cached_tokens::{CachedTokenEvidence, HitCohort};
use crate::measurement::TtftSource;
use crate::record::Phase;
use crate::stats::DistSummary;
use serde::Serialize;
use std::collections::BTreeMap;

pub struct CohortSample<'a> {
    pub phase: Phase,
    pub success: bool,
    pub valid: Option<bool>,
    pub evidence: &'a CachedTokenEvidence,
    pub headline_ttft_s: Option<f64>,
    pub headline_source: Option<TtftSource>,
    pub visible_answer_ttft_s: Option<f64>,
    pub turn: Option<usize>,
    pub prefix_group: Option<&'a str>,
}

#[derive(Debug, Serialize)]
pub struct CohortSummary {
    pub schema_version: &'static str,
    pub phase: &'static str,
    pub measured_requests: usize,
    pub warmup_excluded: usize,
    pub drain_excluded: usize,
    pub errors_excluded_from_timing: usize,
    pub invalid_excluded_from_timing: usize,
    pub counts: BTreeMap<HitCohort, usize>,
    pub groups: Vec<CohortGroup>,
}

#[derive(Debug, Serialize)]
pub struct CohortGroup {
    pub cohort: HitCohort,
    /// Retain source turn numbering; never silently reindex selected boundaries.
    pub turn: Option<usize>,
    pub prefix_group: Option<String>,
    pub requests: usize,
    pub timing_eligible_requests: usize,
    /// Headline first-any resolution grouped by provenance, including explicit
    /// first_byte_approx and unprovenanced groups, never mixed with stream times.
    pub headline_ttft_by_source_s: BTreeMap<String, DistSummary>,
    pub visible_answer_ttft_s: DistSummary,
    pub missing_headline_ttft: usize,
    pub missing_visible_ttft: usize,
    pub invalid_timing_values: usize,
}

#[derive(Default)]
struct Accumulator {
    requests: usize,
    eligible: usize,
    by_source: BTreeMap<String, Vec<f64>>,
    visible: Vec<f64>,
    missing_headline: usize,
    missing_visible: usize,
    invalid_values: usize,
}

pub fn summarize_cohorts<'a>(samples: impl IntoIterator<Item = CohortSample<'a>>) -> CohortSummary {
    let mut summary = CohortSummary {
        schema_version: "cached-token-cohorts.v1",
        phase: "measure",
        measured_requests: 0,
        warmup_excluded: 0,
        drain_excluded: 0,
        errors_excluded_from_timing: 0,
        invalid_excluded_from_timing: 0,
        counts: [
            HitCohort::Cold,
            HitCohort::Partial,
            HitCohort::Full,
            HitCohort::Unknown,
        ]
        .into_iter()
        .map(|cohort| (cohort, 0))
        .collect(),
        groups: vec![],
    };
    let mut groups: BTreeMap<(HitCohort, Option<usize>, Option<String>), Accumulator> =
        BTreeMap::new();
    for sample in samples {
        match sample.phase {
            Phase::Warmup => {
                summary.warmup_excluded += 1;
                continue;
            }
            Phase::Drain => {
                summary.drain_excluded += 1;
                continue;
            }
            Phase::Measure => {}
        }
        summary.measured_requests += 1;
        let eligible = sample.success && sample.valid != Some(false);
        if !sample.success {
            summary.errors_excluded_from_timing += 1;
        } else if !eligible {
            summary.invalid_excluded_from_timing += 1;
        }
        let cohort = if eligible {
            sample.evidence.cohort()
        } else {
            HitCohort::Unknown
        };
        *summary.counts.entry(cohort).or_default() += 1;
        let group = groups
            .entry((cohort, sample.turn, sample.prefix_group.map(str::to_owned)))
            .or_default();
        group.requests += 1;
        if !eligible {
            continue;
        }
        group.eligible += 1;
        match sample.headline_ttft_s {
            Some(value) if value.is_finite() && value >= 0.0 => group
                .by_source
                .entry(
                    sample
                        .headline_source
                        .map(TtftSource::as_str)
                        .unwrap_or("unprovenanced")
                        .into(),
                )
                .or_default()
                .push(value),
            Some(_) => group.invalid_values += 1,
            None => group.missing_headline += 1,
        }
        match sample.visible_answer_ttft_s {
            Some(value) if value.is_finite() && value >= 0.0 => group.visible.push(value),
            Some(_) => group.invalid_values += 1,
            None => group.missing_visible += 1,
        }
    }
    summary.groups = groups
        .into_iter()
        .map(|((cohort, turn, prefix_group), group)| CohortGroup {
            cohort,
            turn,
            prefix_group,
            requests: group.requests,
            timing_eligible_requests: group.eligible,
            headline_ttft_by_source_s: group
                .by_source
                .into_iter()
                .map(|(source, values)| (source, DistSummary::from_values(&values)))
                .collect(),
            visible_answer_ttft_s: DistSummary::from_values(&group.visible),
            missing_headline_ttft: group.missing_headline,
            missing_visible_ttft: group.missing_visible,
            invalid_timing_values: group.invalid_values,
        })
        .collect();
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample(evidence: &CachedTokenEvidence) -> CohortSample<'_> {
        CohortSample {
            phase: Phase::Measure,
            success: true,
            valid: None,
            evidence,
            headline_ttft_s: Some(1.0),
            headline_source: Some(TtftSource::Stream),
            visible_answer_ttft_s: Some(2.0),
            turn: Some(3),
            prefix_group: Some("group-a"),
        }
    }

    #[test]
    fn population_reconciles_without_errors_warmup_or_invalid_in_distributions() {
        let reported = |cached| {
            CachedTokenEvidence::from_usage(
                &json!({"prompt_tokens":8,"prompt_tokens_details":{"cached_tokens":cached}}),
            )
        };
        let cold = reported(0);
        let partial = reported(3);
        let full = reported(8);
        let unknown = CachedTokenEvidence::default();
        let mut warmup = sample(&full);
        warmup.phase = Phase::Warmup;
        let mut drain = sample(&full);
        drain.phase = Phase::Drain;
        let mut error = sample(&full);
        error.success = false;
        error.headline_ttft_s = Some(999.0);
        let mut invalid = sample(&full);
        invalid.valid = Some(false);
        invalid.headline_ttft_s = Some(888.0);
        let summary = summarize_cohorts([
            warmup,
            drain,
            sample(&cold),
            sample(&partial),
            sample(&full),
            sample(&unknown),
            error,
            invalid,
        ]);
        assert_eq!(summary.measured_requests, 6);
        assert_eq!(summary.warmup_excluded, 1);
        assert_eq!(summary.drain_excluded, 1);
        assert_eq!(summary.errors_excluded_from_timing, 1);
        assert_eq!(summary.invalid_excluded_from_timing, 1);
        assert_eq!(
            summary.counts.values().sum::<usize>(),
            summary.measured_requests
        );
        assert_eq!(summary.counts[&HitCohort::Unknown], 3);
        assert_eq!(
            summary
                .groups
                .iter()
                .map(|group| group.requests)
                .sum::<usize>(),
            6
        );
        assert_eq!(
            summary
                .groups
                .iter()
                .map(|group| group.timing_eligible_requests)
                .sum::<usize>(),
            4
        );
        for group in &summary.groups {
            assert_eq!(group.visible_answer_ttft_s.max, Some(2.0));
            assert_eq!(group.headline_ttft_by_source_s["stream"].max, Some(1.0));
            assert_eq!(group.turn, Some(3));
        }
    }

    #[test]
    fn reasoning_first_visible_and_http_approximation_remain_distinct() {
        let evidence = CachedTokenEvidence::from_usage(
            &json!({"prompt_tokens":8,"prompt_tokens_details":{"cached_tokens":8}}),
        );
        let mut reasoning = sample(&evidence);
        reasoning.headline_source = Some(TtftSource::FirstTokenFallback);
        let mut approximate = sample(&evidence);
        approximate.headline_source = Some(TtftSource::FirstByteApprox);
        approximate.headline_ttft_s = Some(3.0);
        approximate.visible_answer_ttft_s = None;
        let mut bad = sample(&evidence);
        bad.headline_ttft_s = Some(f64::NAN);
        bad.visible_answer_ttft_s = Some(-1.0);
        let summary = summarize_cohorts([reasoning, approximate, bad]);
        let group = &summary.groups[0];
        assert_eq!(
            group.headline_ttft_by_source_s["first_token_fallback"].p50,
            Some(1.0)
        );
        assert_eq!(
            group.headline_ttft_by_source_s["first_byte_approx"].p50,
            Some(3.0)
        );
        assert_eq!(group.visible_answer_ttft_s.n, 1);
        assert_eq!(group.visible_answer_ttft_s.p50, Some(2.0));
        assert_eq!(group.missing_visible_ttft, 1);
        assert_eq!(group.invalid_timing_values, 2);
    }
}
