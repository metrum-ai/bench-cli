// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
//! Conditional paired RUN means, never bootstrap correlated individual turns.
use crate::{
    kv_study_export::StudyExport,
    measurement::TtftSource,
    stats::{bootstrap_mean_ci, ConfidenceInterval, DistSummary},
};
use serde::Serialize;
use std::collections::BTreeSet;
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StudyClock {
    Headline(TtftSource),
    VisibleAnswer,
}
pub struct RunPair<'a> {
    pub pair_key: &'a str,
    pub baseline: &'a StudyExport,
    pub baseline_phase: usize,
    pub treatment: &'a StudyExport,
    pub treatment_phase: usize,
}
#[derive(Serialize)]
pub struct PairedComparison {
    schema_version: &'static str,
    statistic: &'static str,
    unit: &'static str,
    clock: StudyClock,
    supplied_pairs: usize,
    included_pairs: usize,
    small_cluster_warning: bool,
    independence: &'static str,
    fixed_controls_sha256: Option<String>,
    differences: Option<DistSummary>,
    ci95: Option<ConfidenceInterval>,
    unknown: Option<&'static str>,
    physical_interpretation: &'static str,
}
/// `controls_digest` is an explicit OPERATOR DECLARATION of fixed/disclosed
/// controls and independence of run pairs, not an executable/reset attestation.
/// Every supplied pair must be distinct, complete and use the same clock and
/// reuse kind. Missing pairs never silently reduce the bootstrap population.
pub fn paired_run_mean_difference(
    pairs: &[RunPair<'_>],
    clock: StudyClock,
    controls_digest: Option<&str>,
    seed: u64,
) -> PairedComparison {
    let mut out=PairedComparison{schema_version:"kv-study-paired-runs.v1",statistic:"treatment_minus_baseline_mean_ttft_per_run",unit:"seconds",clock,supplied_pairs:pairs.len(),included_pairs:0,small_cluster_warning:pairs.len()<20,independence:"not_declared",fixed_controls_sha256:None,differences:None,ci95:None,unknown:None,physical_interpretation:"logical arms only; does not certify cold/resident/external state, reuse equivalence, quality or reset success"};
    let evaluate = || -> Result<Vec<f64>, &'static str> {
        let digest = controls_digest.ok_or("independent_run_controls_not_declared")?;
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("invalid_fixed_controls_digest");
        }
        if !(2..=1000).contains(&pairs.len()) {
            return Err("insufficient_or_excessive_run_pairs");
        }
        let mut keys = BTreeSet::new();
        let mut arms = BTreeSet::new();
        let mut runs = BTreeSet::new();
        let mut differences = Vec::new();
        let mut reuse = None;
        for pair in pairs {
            if pair.pair_key.is_empty() || pair.pair_key.len() > 256 || !keys.insert(pair.pair_key)
            {
                return Err("duplicate_or_invalid_run_pair");
            }
            // Reject reusing the same export arm to inflate cluster n. Collection
            // independence itself remains conditional on the declaration.
            if !arms.insert((
                pair.baseline as *const StudyExport as usize,
                pair.baseline_phase,
            )) || !arms.insert((
                pair.treatment as *const StudyExport as usize,
                pair.treatment_phase,
            )) {
                return Err("reused_export_arm");
            }
            let exports = BTreeSet::from([
                pair.baseline as *const StudyExport as usize,
                pair.treatment as *const StudyExport as usize,
            ]);
            if exports.into_iter().any(|key| !runs.insert(key)) {
                return Err("export_reused_across_run_clusters");
            }
            let kind = pair.baseline.reuse_kind();
            if kind != pair.treatment.reuse_kind() || reuse.is_some_and(|r| r != kind) {
                return Err("incomparable_reuse_kinds");
            }
            reuse = Some(kind);
            let baseline = pair
                .baseline
                .phase_clock_mean(pair.baseline_phase, clock)
                .ok_or("missing_or_invalid_matched_clock")?;
            let treatment = pair
                .treatment
                .phase_clock_mean(pair.treatment_phase, clock)
                .ok_or("missing_or_invalid_matched_clock")?;
            let difference = treatment - baseline;
            if !difference.is_finite() {
                return Err("nonfinite_paired_difference");
            }
            differences.push(difference);
        }
        Ok(differences)
    };
    match evaluate() {
        Ok(values) => {
            let summary = DistSummary::from_values(&values);
            let ci = bootstrap_mean_ci(&values, 0.95, 10_000, seed);
            if [
                summary.min,
                summary.max,
                summary.avg,
                summary.std,
                summary.mad,
                summary.p50,
                summary.p90,
                summary.p95,
                summary.p99,
                ci.low,
                ci.high,
            ]
            .iter()
            .any(|v| v.is_none_or(|n| !n.is_finite()))
            {
                out.unknown = Some("nonfinite_run_cluster_statistics");
            } else {
                out.included_pairs = values.len();
                out.independence = "operator_declared_unverified";
                out.fixed_controls_sha256 = controls_digest.map(str::to_owned);
                out.differences = Some(summary);
                out.ci95 = Some(ci);
            }
        }
        Err(reason) => out.unknown = Some(reason),
    }
    out
}
