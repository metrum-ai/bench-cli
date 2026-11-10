// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Runtime adapters for the versioned cohort model. Full study exports consume
//! the serializable model independently of the legacy summary record schemas.

use crate::cache_cohorts::{summarize_cohorts, CohortSample, CohortSummary};
use crate::cached_tokens::{CachedTokenEvidence, HitCohort};
use crate::record::{Phase, RequestRecord};
use crate::strategic::BenchRecord;
use std::fmt::Write;

pub fn request_cohorts(records: &[RequestRecord]) -> CohortSummary {
    let unknown = CachedTokenEvidence::default();
    summarize_cohorts(records.iter().map(|row| CohortSample {
        phase: row.phase,
        success: row.is_success(),
        valid: None,
        evidence: row.cache_evidence.as_ref().unwrap_or(&unknown),
        headline_ttft_s: row.ttft_first_any_s.or(row.ttft_s),
        headline_source: row.ttft_source,
        visible_answer_ttft_s: row.ttft_visible_answer_s,
        turn: None,
        prefix_group: None,
    }))
}

pub fn strategic_cohorts<'a>(records: impl IntoIterator<Item = &'a BenchRecord>) -> CohortSummary {
    let unknown = CachedTokenEvidence::default();
    summarize_cohorts(records.into_iter().map(|row| CohortSample {
        phase: if row.warmup {
            Phase::Warmup
        } else {
            Phase::Measure
        },
        success: row.success,
        valid: row.valid,
        evidence: row.cache_evidence.as_ref().unwrap_or(&unknown),
        headline_ttft_s: row.first_any_s().or(row.ttft_s),
        headline_source: row.ttft_source,
        visible_answer_ttft_s: row.ttft_visible_answer_s,
        turn: row.turn,
        prefix_group: None,
    }))
}

/// Bounded human overview, never average per-group percentiles into a false
/// global percentile. Exact per-group serializable statistics remain available.
pub fn format_overview(summary: &CohortSummary) -> String {
    let mut text = format!("Cache cohorts (server token reports; measure only; no tier attribution): cold={} partial={} full={} unknown={} measured={}\n",
        summary.counts[&HitCohort::Cold], summary.counts[&HitCohort::Partial], summary.counts[&HitCohort::Full], summary.counts[&HitCohort::Unknown], summary.measured_requests);
    let _ = writeln!(
        text,
        "  Excluded: warmup={} drain={} error timing={} invalid timing={}",
        summary.warmup_excluded,
        summary.drain_excluded,
        summary.errors_excluded_from_timing,
        summary.invalid_excluded_from_timing
    );
    let value = |value: Option<f64>| {
        value
            .map(|value| format!("{value:.6}s"))
            .unwrap_or_else(|| "unknown".into())
    };
    for group in summary.groups.iter().take(8) {
        let _ = writeln!(text, "  {:?} turn={:?}: requests={} timing_eligible={} visible_n={} visible_p50={} visible_p95={}",
            group.cohort, group.turn, group.requests, group.timing_eligible_requests,
            group.visible_answer_ttft_s.n, value(group.visible_answer_ttft_s.p50), value(group.visible_answer_ttft_s.p95));
        for (source, dist) in &group.headline_ttft_by_source_s {
            let _ = writeln!(
                text,
                "    headline source={source} n={} p50={} p95={}",
                dist.n,
                value(dist.p50),
                value(dist.p95)
            );
        }
    }
    if summary.groups.len() > 8 {
        let _ = writeln!(
            text,
            "  {} additional groups omitted from console overview",
            summary.groups.len() - 8
        );
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_cache_and_timing_stay_unknown_in_runtime_adapter() {
        let row = BenchRecord {
            success: true,
            warmup: false,
            turn: Some(7),
            ..Default::default()
        };
        let summary = strategic_cohorts(&[row]);
        assert_eq!(summary.counts[&HitCohort::Unknown], 1);
        assert_eq!(summary.groups[0].turn, Some(7));
        let text = format_overview(&summary);
        assert!(text.contains("visible_p50=unknown"));
        assert!(text.contains("visible_n=0"));
        assert!(!text.contains("visible_p50=0.000000s"));
    }
}
