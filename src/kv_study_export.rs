// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Body-free exports bound to actual measured rows, not manifest-declared counts.
//! This conservative adapter cannot promote declarations to physical/reset proof.
use crate::{
    cache_cohorts::CohortSummary,
    cache_report::{request_cohorts, strategic_cohorts},
    cached_tokens::HitCohort,
    record::RequestRecord,
    strategic::BenchRecord,
};
use anyhow::{bail, ensure, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};

pub struct StudySeed(Value);
impl StudySeed {
    pub(crate) fn check_controls(
        &self,
        policy: crate::cache_workload::CachePolicy,
        order: crate::cache_workload::OrderPolicy,
    ) -> Result<()> {
        ensure!(
            self.0["workload"]["cache_policy"] == serde_json::to_value(policy)?,
            "study_cache_policy_mismatch"
        );
        ensure!(
            self.0["workload"]["order"] == serde_json::to_value(order)?,
            "study_effective_order_mismatch"
        );
        Ok(())
    }
    pub(crate) fn measured_phase_ids(&self) -> Result<Vec<String>> {
        self.0["phases"]
            .as_array()
            .expect("validated phases")
            .iter()
            .map(|p| {
                ensure!(p["kind"] != "warmup", "study_warmup_adapter_required");
                Ok(p["id"].as_str().expect("validated id").to_owned())
            })
            .collect()
    }
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= 4 * 1024 * 1024, "study_manifest_size_limit");
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|_| anyhow::anyhow!("study_manifest_invalid_json"))?;
        static VALIDATOR: OnceLock<jsonschema::Validator> = OnceLock::new();
        let validator = VALIDATOR.get_or_init(|| {
            jsonschema::validator_for(
                &serde_json::from_str(include_str!("../schemas/kv-study-report.v1.json"))
                    .expect("bundled schema JSON"),
            )
            .expect("bundled study schema")
        });
        ensure!(validator.is_valid(&value), "study_manifest_schema_invalid");
        let phases = value["phases"].as_array().expect("validated array");
        ensure!(
            !phases.is_empty() && phases.len() <= 1000,
            "study_phase_limit"
        );
        let mut ids = BTreeSet::new();
        for phase in phases {
            ensure!(
                ids.insert(phase["id"].as_str().expect("validated id")),
                "study_duplicate_phase_id"
            );
            ensure!(
                phase["cache_state"] == "unknown",
                "study_unbound_physical_claim"
            );
            for key in ["reset", "pre_warmup_reset"] {
                if let Some(reset) = phase.get(key) {
                    ensure!(reset["status"] != "verified", "study_unbound_reset_claim");
                }
            }
        }
        for tier in value["runtime"]["tiers"]
            .as_array()
            .expect("validated tiers")
        {
            ensure!(tier["evidence"] != "observed", "study_unbound_tier_claim");
        }
        Ok(Self(value))
    }
}
pub enum PhaseRecords<'a> {
    Requests(&'a [RequestRecord]),
    Strategic(&'a [BenchRecord]),
    StrategicRefs(&'a [&'a BenchRecord]),
}
impl PhaseRecords<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Requests(r) => r.len(),
            Self::Strategic(r) => r.len(),
            Self::StrategicRefs(r) => r.len(),
        }
    }
    fn addresses(&self) -> Vec<(u8, usize)> {
        match self {
            Self::Requests(r) => r
                .iter()
                .map(|v| (0, v as *const RequestRecord as usize))
                .collect(),
            Self::StrategicRefs(r) => r
                .iter()
                .map(|v| (1, *v as *const BenchRecord as usize))
                .collect(),
            Self::Strategic(r) => r
                .iter()
                .map(|v| (1, v as *const BenchRecord as usize))
                .collect(),
        }
    }
    fn final_bodies(&self) -> Vec<crate::kv_study_bindings::ObservedFinalBody> {
        let mut rows: Vec<_> = match self {
            Self::Requests(r) => r
                .iter()
                .filter(|r| r.phase == crate::record::Phase::Measure)
                .map(|r| {
                    (
                        r.seq,
                        r.modality_labels.get("body_sha256").map(String::as_str),
                        r.modality_labels
                            .get("cache_policy_effects")
                            .map(String::as_str),
                    )
                })
                .collect(),
            Self::Strategic(r) => r
                .iter()
                .filter(|r| !r.warmup)
                .map(|r| {
                    (
                        r.seq,
                        r.body_sha256.as_deref(),
                        r.cache_policy_effects.as_deref(),
                    )
                })
                .collect(),
            Self::StrategicRefs(r) => r
                .iter()
                .filter(|r| !r.warmup)
                .map(|r| {
                    (
                        r.seq,
                        r.body_sha256.as_deref(),
                        r.cache_policy_effects.as_deref(),
                    )
                })
                .collect(),
        };
        rows.sort_by_key(|r| r.0);
        rows.into_iter()
            .enumerate()
            .map(|(i, (_, hash, effects))| {
                crate::kv_study_bindings::ObservedFinalBody::project(i + 1, hash, effects)
            })
            .collect()
    }
    fn delays(&self) -> ScheduledDelayView {
        let mut queue = Vec::new();
        let mut corrected = Vec::new();
        let mut missing = 0;
        let mut add = |scheduled: bool, latency: f64, delay: f64| {
            if scheduled
                && latency.is_finite()
                && latency >= 0.0
                && delay.is_finite()
                && delay >= 0.0
                && (latency + delay).is_finite()
            {
                queue.push(delay);
                corrected.push(latency + delay);
            } else {
                missing += 1;
            }
        };
        match self {
            Self::Requests(rows) => {
                for r in *rows {
                    if r.phase == crate::record::Phase::Measure && r.is_success() {
                        add(
                            r.scheduled_offset_s
                                .is_some_and(|s| s.is_finite() && s >= 0.0),
                            r.latency_s,
                            r.queue_delay_s,
                        );
                    }
                }
            }
            Self::Strategic(rows) => {
                for r in *rows {
                    if !r.warmup && r.success && r.valid != Some(false) {
                        add(r.scheduled_unix_ns > 0, r.latency_s, r.queue_delay_s);
                    }
                }
            }
            Self::StrategicRefs(rows) => {
                for r in *rows {
                    if !r.warmup && r.success && r.valid != Some(false) {
                        add(r.scheduled_unix_ns > 0, r.latency_s, r.queue_delay_s);
                    }
                }
            }
        }
        ScheduledDelayView {
            unit: "seconds",
            scheduled_rows: queue.len(),
            missing_or_invalid_rows: missing,
            client_queue_delay_s: crate::stats::DistSummary::from_values(&queue),
            scheduled_arrival_latency_s: crate::stats::DistSummary::from_values(&corrected),
            cap_engagement: "unknown; delay alone is not cap/backlog engagement evidence",
        }
    }
    fn cohorts(&self) -> CohortSummary {
        match self {
            Self::Requests(r) => request_cohorts(r),
            Self::Strategic(r) => strategic_cohorts(*r),
            Self::StrategicRefs(r) => strategic_cohorts(r.iter().copied()),
        }
    }
}
#[derive(Serialize)]
struct ScheduledDelayView {
    unit: &'static str,
    scheduled_rows: usize,
    missing_or_invalid_rows: usize,
    client_queue_delay_s: crate::stats::DistSummary,
    scheduled_arrival_latency_s: crate::stats::DistSummary,
    cap_engagement: &'static str,
}
#[derive(Serialize)]
pub struct PhaseView {
    phase_id: String,
    phase_label: String,
    kind: String,
    cache_state: &'static str,
    requests: usize,
    errors: usize,
    cached_token_known: usize,
    cached_token_unknown: usize,
    ttft_unit: &'static str,
    uncertainty: &'static str,
    small_n_warning: bool,
    cohorts: CohortSummary,
    scheduled_delays: ScheduledDelayView,
    native_window: Value,
    observed_final_bodies: Vec<crate::kv_study_bindings::ObservedFinalBody>,
}
#[derive(Serialize)]
pub struct StudyExport {
    schema_version: &'static str,
    report: Value,
    phases: Vec<PhaseView>,
    engineering_non_certifying: bool,
    metadata_provenance: &'static str,
    publication_blockers: Vec<&'static str>,
    comparisons: Vec<crate::kv_study_comparison::PairedComparison>,
    runtime_bindings: Value,
}
impl StudyExport {
    /// Bind each declared measured phase exactly once. Warmup/drain rows remain
    /// excluded by the existing authoritative cohort adapters. Warmup study
    /// phases require a separate adapter; never relabel them measured for TTFT.
    pub fn bind(
        mut seed: StudySeed,
        bindings: &BTreeMap<String, PhaseRecords<'_>>,
    ) -> Result<Self> {
        let declared = seed.0["phases"].as_array_mut().expect("validated phases");
        ensure!(
            bindings.len() == declared.len(),
            "study_phase_binding_mismatch"
        );
        let total = bindings
            .values()
            .try_fold(0usize, |n, b| n.checked_add(b.len()))
            .ok_or_else(|| anyhow::anyhow!("study_record_limit"))?;
        ensure!(total <= 100_000, "study_record_limit");
        // Detect aliasing of supplied rows across phases. This is not an
        // independent attestation of collection identity or executable state.
        let mut identities = BTreeSet::new();
        for rows in bindings.values() {
            for identity in rows.addresses() {
                ensure!(identities.insert(identity), "study_duplicate_supplied_row");
            }
        }
        let mut phases = Vec::new();
        for (index, phase) in declared.iter_mut().enumerate() {
            if phase["kind"] == "warmup" {
                bail!("study_warmup_adapter_required");
            }
            let id = phase["id"].as_str().expect("validated id").to_owned();
            let rows = bindings
                .get(&id)
                .ok_or_else(|| anyhow::anyhow!("study_phase_binding_missing"))?;
            let cohorts = rows.cohorts();
            ensure!(cohorts.groups.len() <= 10_000, "study_group_limit");
            let requests = cohorts.measured_requests;
            let errors = cohorts.errors_excluded_from_timing;
            let unknown = cohorts.counts[&HitCohort::Unknown];
            let known = requests
                .checked_sub(unknown)
                .ok_or_else(|| anyhow::anyhow!("study_count_invariant"))?;
            phase["requests"] = json!(requests);
            phase["errors"] = json!(errors);
            phase["cached_token_known"] = json!(known);
            phase["cached_token_unknown"] = json!(unknown);
            let small = cohorts.groups.is_empty()
                || cohorts.groups.iter().any(|g| {
                    g.visible_answer_ttft_s.p95_unreliable
                        || g.headline_ttft_by_source_s
                            .values()
                            .any(|d| d.p95_unreliable)
                });
            phases.push(PhaseView {
                phase_id: format!("phase-{}", index + 1),
                phase_label: format!("phase: {id}"),
                kind: phase["kind"].as_str().expect("validated kind").into(),
                cache_state: "unknown",
                requests,
                errors,
                cached_token_known: known,
                cached_token_unknown: unknown,
                ttft_unit: "seconds",
                uncertainty: "unknown: independent run/session cluster evidence not supplied",
                small_n_warning: small,
                observed_final_bodies:rows.final_bodies(),
                scheduled_delays:rows.delays(),
                native_window:json!({"schema_version":"cache-phase-analysis.v1","unknown":"not_requested_or_not_executed","physical_cache_state":"unknown"}),
                cohorts,
            });
        }
        seed.0["limitations"].as_array_mut().expect("validated limitations").push(json!("Runtime metadata and reset declarations are not independently verified. Cache state, freshness, physical tier participation, saved-prefill tokens and cluster uncertainty remain unknown. Request cached-token reports are not theoretical-overlap or approximate/compressed-reuse proof."));
        Ok(Self{schema_version:"kv-study-export.v1",report:seed.0,phases,engineering_non_certifying:true,metadata_provenance:"manifest declarations are unverified; phase counts/cohorts bound to supplied runtime rows",publication_blockers:vec!["physical_cache_state_unverified","runtime_and_reset_provenance_unbound","paired_cluster_repetitions_not_independently_verified","warmup_neutrality_not_independently_verified"],comparisons:Vec::new(),runtime_bindings:json!({"schema_version":"kv-study-runtime-bindings.v1","unknown":"not_bound"})})
    }
    pub(crate) fn reuse_kind(&self) -> &str {
        self.report["reuse_kind"].as_str().expect("validated kind")
    }
    pub(crate) fn matching_population(
        &self,
        index: usize,
        other: &Self,
        other_index: usize,
    ) -> bool {
        let population = |phase: &PhaseView| {
            let mut counts = BTreeMap::new();
            for group in &phase.cohorts.groups {
                *counts
                    .entry((group.turn, group.prefix_group.clone()))
                    .or_insert(0usize) += group.timing_eligible_requests;
            }
            counts
        };
        self.phases
            .get(index)
            .zip(other.phases.get(other_index))
            .is_some_and(|(a, b)| population(a) == population(b))
    }
    pub(crate) fn phase_clock_mean(
        &self,
        index: usize,
        clock: crate::kv_study_comparison::StudyClock,
    ) -> Option<f64> {
        let phase = self.phases.get(index)?;
        if phase.cohorts.errors_excluded_from_timing > 0
            || phase.cohorts.invalid_excluded_from_timing > 0
        {
            return None;
        }
        let mut n = 0usize;
        let mut total = 0.0;
        for group in &phase.cohorts.groups {
            let dist = match clock {
                crate::kv_study_comparison::StudyClock::VisibleAnswer => {
                    Some(&group.visible_answer_ttft_s)
                }
                crate::kv_study_comparison::StudyClock::Headline(source) => {
                    group.headline_ttft_by_source_s.get(source.as_str())
                }
            };
            if group.timing_eligible_requests > 0
                && dist.is_none_or(|d| d.n != group.timing_eligible_requests)
            {
                return None;
            }
            if let Some(dist) = dist {
                if dist.n > 0 {
                    let mean = dist.avg?;
                    if !mean.is_finite() {
                        return None;
                    }
                    n = n.checked_add(dist.n)?;
                    total += mean * dist.n as f64;
                }
            }
        }
        let mean = total / n as f64;
        (n > 0 && mean.is_finite()).then_some(mean)
    }
    pub fn add_comparison(
        &mut self,
        comparison: crate::kv_study_comparison::PairedComparison,
    ) -> Result<()> {
        ensure!(self.comparisons.len() < 1000, "study_comparison_limit");
        self.comparisons.push(comparison);
        Ok(())
    }
    pub(crate) fn attach_runtime(&mut self, bindings: &Value) -> Result<()> {
        ensure!(
            bindings["schema_version"] == "kv-study-runtime-bindings.v1",
            "study_runtime_binding_version_mismatch"
        );
        self.runtime_bindings = bindings.clone();
        Ok(())
    }
    pub(crate) fn attach_native(&mut self, windows: &[Value]) -> Result<()> {
        ensure!(
            windows.len() <= self.phases.len(),
            "study_native_window_mapping_mismatch"
        );
        for (phase, window) in self.phases.iter_mut().zip(windows) {
            ensure!(
                window["schema_version"] == "cache-phase-analysis.v1"
                    && window["physical_cache_state"] == "unknown",
                "study_native_window_contract_mismatch"
            );
            phase.native_window = window.clone();
        }
        Ok(())
    }
    pub fn json(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec_pretty(self)?)
    }
    /// One row per phase; detailed cohort timing remains a JSON cell, preserving
    /// source/turn populations rather than inventing a merged percentile.
    pub fn csv(&self) -> Result<Vec<u8>> {
        let mut writer = csv::Writer::from_writer(Vec::new());
        writer.write_record([
            "schema_version",
            "phase_id",
            "phase_label",
            "kind",
            "cache_state",
            "requests",
            "errors",
            "cached_token_known",
            "cached_token_unknown",
            "ttft_unit",
            "uncertainty",
            "small_n_warning",
            "cohorts_json",
            "scheduled_delays_json",
            "native_window_json",
            "run_comparisons_json",
            "observed_final_bodies_json",
            "runtime_bindings_json",
        ])?;
        for row in &self.phases {
            writer.write_record([
                self.schema_version.into(),
                row.phase_id.clone(),
                row.phase_label.clone(),
                row.kind.clone(),
                row.cache_state.into(),
                row.requests.to_string(),
                row.errors.to_string(),
                row.cached_token_known.to_string(),
                row.cached_token_unknown.to_string(),
                row.ttft_unit.into(),
                row.uncertainty.into(),
                row.small_n_warning.to_string(),
                serde_json::to_string(&row.cohorts)?,
                serde_json::to_string(&row.scheduled_delays)?,
                serde_json::to_string(&row.native_window)?,
                serde_json::to_string(&self.comparisons)?,
                serde_json::to_string(&row.observed_final_bodies)?,
                serde_json::to_string(&self.runtime_bindings)?,
            ])?;
        }
        Ok(writer.into_inner()?)
    }
    /// Embed the same canonical phase view as escaped text, never executable
    /// script/markup supplied by study identifiers or metadata.
    pub fn html(&self) -> Result<String> {
        let escape = |s: &str| {
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
                .replace('\'', "&#39;")
        };
        let mut out=String::from("<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>Engineering KV study</title><h1>Non-certifying KV study</h1><p>Unknown physical state is not cold; generic warmup is not cache-neutral. Uncertainty is unknown without independent paired cluster evidence. Small-n warnings retained.</p>");
        let number = |v: Option<f64>| v.map(|v| v.to_string()).unwrap_or_else(|| "unknown".into());
        for row in &self.phases {
            out.push_str(&format!("<section><h2>{}</h2><p>Logical arm: {}; physical state: unknown. Counts are supplied measured rows, not planned requests.</p><table><caption>Observed request counts</caption><tr><th>Requests</th><th>Errors</th><th>Cached-token known</th><th>Cached-token unknown</th></tr><tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr></table>",escape(&row.phase_label),escape(&row.kind),row.requests,row.errors,row.cached_token_known,row.cached_token_unknown));
            out.push_str("<table><caption>TTFT seconds — server-reported cache cohorts, not physical cache states. Clock/turn populations remain separate; type-7 percentiles.</caption><tr><th>Cohort / turn</th><th>Clock / provenance</th><th>n</th><th>p50 s</th><th>p95 s</th><th>Small-n p95 warning</th></tr>");
            for group in &row.cohorts.groups {
                let label = escape(&format!("{:?} / {:?}", group.cohort, group.turn));
                for (source, dist) in &group.headline_ttft_by_source_s {
                    out.push_str(&format!("<tr><td>{label}</td><td>headline / {}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",escape(source),dist.n,number(dist.p50),number(dist.p95),dist.p95_unreliable));
                }
                let dist = &group.visible_answer_ttft_s;
                out.push_str(&format!("<tr><td>{label}</td><td>visible answer</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",dist.n,number(dist.p50),number(dist.p95),dist.p95_unreliable));
            }
            out.push_str("</table><table><caption>Client scheduling evidence, seconds; no inferred cap engagement</caption><tr><th>Metric</th><th>n</th><th>p50 s</th><th>p95 s</th><th>Small-n p95 warning</th></tr>");
            for (name, dist) in [
                (
                    "client queue delay",
                    &row.scheduled_delays.client_queue_delay_s,
                ),
                (
                    "scheduled-arrival latency (queue + request)",
                    &row.scheduled_delays.scheduled_arrival_latency_s,
                ),
            ] {
                out.push_str(&format!(
                    "<tr><td>{name}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                    dist.n,
                    number(dist.p50),
                    number(dist.p95),
                    dist.p95_unreliable
                ));
            }
            out.push_str(&format!("</table><p>{}</p><details><summary>Canonical phase evidence, native units/coverage and exclusions</summary><pre>{}</pre></details></section>",escape(row.uncertainty),escape(&serde_json::to_string_pretty(row)?)));
        }
        out.push_str(&format!("<details><summary>Conditional paired run-mean comparisons (not per-turn bootstrap)</summary><pre>{}</pre></details>",escape(&serde_json::to_string_pretty(&self.comparisons)?)));
        out.push_str(&format!("<details><summary>Observed runtime configuration and producer bindings; not physical attestation</summary><pre>{}</pre></details>",escape(&serde_json::to_string_pretty(&self.runtime_bindings)?)));
        out.push_str(&format!("<details><summary>Manifest metadata declarations (unverified)</summary><pre>{}</pre></details></html>",escape(&serde_json::to_string_pretty(&self.report)?)));
        Ok(out)
    }
}
