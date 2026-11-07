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
}
impl PhaseRecords<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Requests(r) => r.len(),
            Self::Strategic(r) => r.len(),
        }
    }
    fn addresses(&self) -> Vec<(u8, usize)> {
        match self {
            Self::Requests(r) => r
                .iter()
                .map(|v| (0, v as *const RequestRecord as usize))
                .collect(),
            Self::Strategic(r) => r
                .iter()
                .map(|v| (1, v as *const BenchRecord as usize))
                .collect(),
        }
    }
    fn cohorts(&self) -> CohortSummary {
        match self {
            Self::Requests(r) => request_cohorts(r),
            Self::Strategic(r) => strategic_cohorts(*r),
        }
    }
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
}
#[derive(Serialize)]
pub struct StudyExport {
    schema_version: &'static str,
    report: Value,
    phases: Vec<PhaseView>,
    engineering_non_certifying: bool,
    metadata_provenance: &'static str,
    publication_blockers: Vec<&'static str>,
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
                cohorts,
            });
        }
        seed.0["limitations"].as_array_mut().expect("validated limitations").push(json!("Runtime metadata and reset declarations are not independently verified. Cache state, freshness, physical tier participation, saved-prefill tokens and cluster uncertainty remain unknown. Request cached-token reports are not theoretical-overlap or approximate/compressed-reuse proof."));
        Ok(Self{schema_version:"kv-study-export.v1",report:seed.0,phases,engineering_non_certifying:true,metadata_provenance:"manifest declarations are unverified; phase counts/cohorts bound to supplied runtime rows",publication_blockers:vec!["physical_cache_state_unverified","runtime_and_reset_provenance_unbound","paired_cluster_repetitions_not_supplied","warmup_neutrality_not_independently_verified"]})
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
        for row in &self.phases {
            out.push_str(&format!(
                "<section><h2>{}</h2><pre>{}</pre></section>",
                escape(&row.phase_label),
                escape(&serde_json::to_string_pretty(row)?)
            ));
        }
        out.push_str("</html>");
        Ok(out)
    }
}
