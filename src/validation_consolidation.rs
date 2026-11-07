// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Metadata-only cross-source consolidation, not another selector or #421 schema.
use crate::prefix_validation::OverlapValidation;
use crate::public_dataset::{sha256, AccessDecision, Evidence, RightsPolicy, ValidationManifest};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Finding {
    Pass,
    Fail,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Check {
    DeclaredAccessPolicy,
    DeclaredComponentCoverage,
    RenderedSample,
    CountConsistency,
    IndependentPrivacyClearance,
    FinalIssuedBody,
    PhysicalCacheObservations,
    SemanticAnswerPatchLeakage,
    RoleToolRequestBoundaries,
    OutputBudgetDistributions,
    DocumentOrderTruncation,
    TraceUnitsHashSemantics,
    DesiredAchievedOrder,
    LiveRecordedHistory,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CountUnit {
    SourceRows,
    Documents,
    DerivedTargets,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CountCompleteness {
    Complete,
    Partial,
    Unavailable,
}

/// Metadata completeness is separate from what a numeric claim purports to count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CountClaim {
    ScopeTotal,
    ObservedSubset,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CountObservation {
    /// Digest of the exact config/split/file scope; unlike units are not compared.
    pub scope_sha256: String,
    pub unit: CountUnit,
    pub count: Evidence<u64>,
    pub completeness: CountCompleteness,
    pub claim: CountClaim,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentReview {
    pub scope_sha256: String,
    pub origin_sha256: String,
    pub selected_ids_sha256: String,
    pub policy: RightsPolicy,
}
impl ComponentReview {
    pub fn bind(manifest: &ValidationManifest, label: &str, policy: RightsPolicy) -> Result<Self> {
        Ok(Self {
            scope_sha256: review_scope_sha256(manifest, label)?,
            origin_sha256: sha256(&serde_json::to_vec(&manifest.origin)?),
            selected_ids_sha256: manifest.selected_ids_sha256.clone(),
            policy,
        })
    }
}
pub struct ConsolidationInput<'a> {
    pub manifest: &'a ValidationManifest,
    pub overlap: Option<&'a OverlapValidation>,
    pub counts: &'a [CountObservation],
    /// Explicit operator declaration, not inferred from source/card/publicity.
    pub required_component_scopes: &'a [String],
    pub component_inventory_complete: bool,
    pub component_reviews: &'a [ComponentReview],
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidatedValidation {
    pub schema_version: String,
    pub validation_scope: String,
    pub manifest_sha256: String,
    pub origin_sha256: String,
    pub rights_policy_sha256: String,
    pub selected_ids_sha256: String,
    pub body_order_sha256: String,
    pub component_review_sha256: String,
    pub selected_source_rows: u64,
    pub targets: u64,
    pub corpus_rows: Evidence<u64>,
    pub counts: Vec<CountObservation>,
    pub count_discrepancies: Vec<[usize; 2]>,
    pub rendered_sample_size: Option<usize>,
    pub checks: BTreeMap<Check, Evidence<Finding>>,
    pub human_summary: String,
    pub report_sha256: String,
}
fn digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
/// Bind a private component/row scope label to exact source and selected IDs.
pub fn review_scope_sha256(manifest: &ValidationManifest, label: &str) -> Result<String> {
    ensure!(
        !label.is_empty() && label.len() <= 4096,
        "invalid private review scope label"
    );
    Ok(sha256(&serde_json::to_vec(&(
        &manifest.origin,
        &manifest.selected_ids_sha256,
        label,
    ))?))
}
fn access(policy: &RightsPolicy) -> Evidence<Finding> {
    if policy.decision == AccessDecision::Denied {
        Evidence::Measured(Finding::Fail)
    } else if policy.authorize().is_ok() {
        Evidence::Measured(Finding::Pass)
    } else {
        Evidence::Unknown
    }
}
fn number(e: &Evidence<u64>) -> Option<u64> {
    match e {
        Evidence::Measured(n) | Evidence::Publisher(n) => Some(*n),
        Evidence::Unknown => None,
    }
}
/// Consolidate explicitly scoped declared evidence. This checks artifact binding,
/// not source bytes again. A convenience rendered sample never certifies corpus
/// quantiles, public availability never establishes privacy/legal clearance, and
/// raw identifiers/license review notes are not emitted by this report.
pub fn consolidate(input: ConsolidationInput<'_>) -> Result<ConsolidatedValidation> {
    let manifest = input.manifest;
    let manifest_sha = manifest.fingerprint()?;
    ensure!(
        input.counts.len() <= 1000
            && input.required_component_scopes.len() <= 10000
            && input.component_reviews.len() <= 10000,
        "consolidation metadata limit"
    );
    let required: BTreeSet<_> = input.required_component_scopes.iter().cloned().collect();
    ensure!(
        required.len() == input.required_component_scopes.len()
            && required.iter().all(|s| digest(s)),
        "invalid/duplicate required component scope"
    );
    let mut reviewed = BTreeMap::new();
    for review in input.component_reviews {
        ensure!(
            digest(&review.scope_sha256)
                && review.origin_sha256 == sha256(&serde_json::to_vec(&manifest.origin)?)
                && review.selected_ids_sha256 == manifest.selected_ids_sha256
                && required.contains(&review.scope_sha256)
                && reviewed
                    .insert(&review.scope_sha256, access(&review.policy))
                    .is_none(),
            "unbound/duplicate component review"
        );
    }
    let known_components_accounted = manifest
        .rights
        .component_terms
        .keys()
        .map(|label| review_scope_sha256(manifest, label))
        .collect::<Result<Vec<_>>>()?
        .iter()
        .all(|scope| required.contains(scope));
    let component = if reviewed
        .values()
        .any(|e| *e == Evidence::Measured(Finding::Fail))
    {
        Evidence::Measured(Finding::Fail)
    } else if known_components_accounted
        && input.component_inventory_complete
        && reviewed.len() == required.len()
        && reviewed
            .values()
            .all(|e| *e == Evidence::Measured(Finding::Pass))
    {
        Evidence::Measured(Finding::Pass)
    } else {
        Evidence::Unknown
    };
    let mut discrepancies = Vec::new();
    let mut comparable_pairs = 0usize;
    for (i, left) in input.counts.iter().enumerate() {
        ensure!(digest(&left.scope_sha256), "invalid count scope digest");
        ensure!(
            left.completeness != CountCompleteness::Unavailable || left.count == Evidence::Unknown,
            "unavailable count must remain unknown, not invented zero"
        );
        for (j, right) in input.counts.iter().enumerate().take(i) {
            if left.scope_sha256 == right.scope_sha256
                && left.unit == right.unit
                && left.claim == CountClaim::ScopeTotal
                && right.claim == CountClaim::ScopeTotal
            {
                if let (Some(a), Some(b)) = (number(&left.count), number(&right.count)) {
                    comparable_pairs += 1;
                    if a != b {
                        discrepancies.push([j, i]);
                    }
                }
            }
        }
    }
    let (rendered, sample) = if let Some(overlap) = input.overlap {
        let artifact_sha = sha256(&serde_json::to_vec(overlap)?);
        let expected = format!("artifact_sha256={artifact_sha}; n={}", overlap.sample_size);
        ensure!(
            manifest.checks.get("rendered_overlap") == Some(&Evidence::Measured(expected)),
            "rendered artifact not bound to manifest"
        );
        ensure!(
            manifest.tokenizer_revision.as_ref() == Some(&overlap.identity.tokenizer_revision)
                && manifest.chat_template_sha256.as_ref()
                    == Some(&overlap.identity.chat_template_sha256)
                && overlap.sample_size as u64 == manifest.target_count
                && overlap.sample_size > 0
                && overlap.block_size > 0,
            "rendered evidence identity/count mismatch"
        );
        (Evidence::Measured(Finding::Pass), Some(overlap.sample_size))
    } else {
        (Evidence::Unknown, None)
    };
    let checks = BTreeMap::from([
        (Check::DeclaredAccessPolicy, access(&manifest.rights)),
        (Check::DeclaredComponentCoverage, component),
        (Check::RenderedSample, rendered),
        (
            Check::CountConsistency,
            if !discrepancies.is_empty() {
                Evidence::Measured(Finding::Fail)
            } else if comparable_pairs == 0
                || input.counts.iter().any(|o| {
                    number(&o.count).is_none()
                        || o.completeness != CountCompleteness::Complete
                        || o.claim != CountClaim::ScopeTotal
                })
            {
                Evidence::Unknown
            } else {
                Evidence::Measured(Finding::Pass)
            },
        ),
        (Check::IndependentPrivacyClearance, Evidence::Unknown),
        (Check::FinalIssuedBody, Evidence::Unknown),
        (Check::PhysicalCacheObservations, Evidence::Unknown),
        (Check::SemanticAnswerPatchLeakage, Evidence::Unknown),
        (Check::RoleToolRequestBoundaries, Evidence::Unknown),
        (Check::OutputBudgetDistributions, Evidence::Unknown),
        (Check::DocumentOrderTruncation, Evidence::Unknown),
        (Check::TraceUnitsHashSemantics, Evidence::Unknown),
        (Check::DesiredAchievedOrder, Evidence::Unknown),
        (Check::LiveRecordedHistory, Evidence::Unknown),
    ]);
    let unknown = checks.values().filter(|e| **e == Evidence::Unknown).count();
    let failed = checks
        .values()
        .filter(|e| **e == Evidence::Measured(Finding::Fail))
        .count();
    let summary=format!("Scoped validation consolidation: {} selected rows, {} targets, {} discrepancies, {} failed checks, {} unknown checks. Rendered evidence is a selected convenience sample, not corpus certification. Declared policy authorization is not independent legal/privacy clearance. No final-issued-body or physical cache observations.",manifest.selected_source_rows,manifest.target_count,discrepancies.len(),failed,unknown);
    let mut report = ConsolidatedValidation {
        schema_version: "source-validation-consolidation.v1".into(),
        validation_scope: "artifact_binding_not_source_reverification".into(),
        manifest_sha256: manifest_sha,
        origin_sha256: sha256(&serde_json::to_vec(&manifest.origin)?),
        rights_policy_sha256: sha256(&serde_json::to_vec(&manifest.rights)?),
        selected_ids_sha256: manifest.selected_ids_sha256.clone(),
        body_order_sha256: manifest.body_order_sha256.clone(),
        component_review_sha256: sha256(&serde_json::to_vec(&(
            input.required_component_scopes,
            input.component_inventory_complete,
            input.component_reviews,
        ))?),
        selected_source_rows: manifest.selected_source_rows,
        targets: manifest.target_count,
        corpus_rows: manifest.corpus_rows.clone(),
        counts: input.counts.to_vec(),
        count_discrepancies: discrepancies,
        rendered_sample_size: sample,
        checks,
        human_summary: summary,
        report_sha256: String::new(),
    };
    let mut value = serde_json::to_value(&report)?;
    value
        .as_object_mut()
        .expect("report object")
        .remove("report_sha256");
    report.report_sha256 = sha256(&serde_json::to_vec(&value)?);
    Ok(report)
}

/// New metadata file only; payload is never included and existing paths are not overwritten.
pub fn write_report(report: &ConsolidatedValidation, path: &std::path::Path) -> Result<()> {
    use std::io::Write;
    crate::public_profile::check_new_outputs(&[path])?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut file, report)?;
    file.write_all(b"\n")?;
    file.flush()?;
    file.persist_noclobber(path).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::public_dataset::{DatasetOrigin, SourceFile, SCHEMA_VERSION};
    fn manifest() -> ValidationManifest {
        ValidationManifest {
            schema_version: SCHEMA_VERSION.into(),
            origin: DatasetOrigin {
                source_id: "example/synthetic".into(),
                resolved_revision: "a".repeat(40),
                config: "test".into(),
                split: "test".into(),
                files: vec![SourceFile {
                    path: "synthetic.jsonl".into(),
                    sha256: sha256(b"data"),
                    bytes: 4,
                }],
            },
            rights: RightsPolicy {
                decision: AccessDecision::Allowed,
                license: Some("Apache-2.0".into()),
                license_text_sha256: Some("b".repeat(64)),
                component_terms: BTreeMap::new(),
                gated: false,
                terms_review: None,
                redistribution_allowed: false,
                privacy_handling: "synthetic PRIVATE-NOTE never public".into(),
                unresolved: vec![],
            },
            adapter_id: "synthetic".into(),
            adapter_version: "1".into(),
            field_mapping: BTreeMap::from([("document".into(), "private-column".into())]),
            transformations: vec![],
            tokenizer_revision: None,
            chat_template_sha256: None,
            generator_version: None,
            corpus_rows: Evidence::Unknown,
            selected_source_rows: 2,
            target_count: 2,
            selected_ids_sha256: "c".repeat(64),
            body_order_sha256: "d".repeat(64),
            checks: BTreeMap::new(),
            exclusions: vec![],
            limitations: vec![],
        }
    }
    fn input(m: &ValidationManifest) -> ConsolidationInput<'_> {
        ConsolidationInput {
            manifest: m,
            overlap: None,
            counts: &[],
            required_component_scopes: &[],
            component_inventory_complete: false,
            component_reviews: &[],
        }
    }
    #[test]
    fn missing_evidence_never_implies_clearance_or_corpus_pass_and_summary_is_private() {
        let m = manifest();
        let r = consolidate(input(&m)).unwrap();
        assert_eq!(r.corpus_rows, Evidence::Unknown);
        for check in [
            Check::DeclaredComponentCoverage,
            Check::RenderedSample,
            Check::CountConsistency,
            Check::IndependentPrivacyClearance,
            Check::FinalIssuedBody,
            Check::PhysicalCacheObservations,
        ] {
            assert_eq!(r.checks[&check], Evidence::Unknown);
        }
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("PRIVATE-NOTE") && !json.contains("private-column"));
        assert_eq!(
            r.report_sha256,
            consolidate(input(&m)).unwrap().report_sha256
        );
        let mut unknown = m.clone();
        unknown.rights.decision = AccessDecision::ReviewRequired;
        assert_eq!(
            consolidate(input(&unknown)).unwrap().checks[&Check::DeclaredAccessPolicy],
            Evidence::Unknown
        );
        unknown.rights.decision = AccessDecision::Denied;
        assert_eq!(
            consolidate(input(&unknown)).unwrap().checks[&Check::DeclaredAccessPolicy],
            Evidence::Measured(Finding::Fail)
        );
    }
    #[test]
    fn report_schema_roundtrip_no_clobber_and_false_physical_claim_rejected() {
        let m = manifest();
        let report = consolidate(input(&m)).unwrap();
        let schema: serde_json::Value = serde_json::from_str(include_str!(
            "../schemas/source-validation-consolidation.v1.json"
        ))
        .unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        let value = serde_json::to_value(&report).unwrap();
        assert!(validator.is_valid(&value));
        let mut false_claim = value;
        false_claim["checks"]["physical_cache_observations"] =
            serde_json::json!({"status":"measured","value":"pass"});
        assert!(!validator.is_valid(&false_claim));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("metadata.json");
        write_report(&report, &path).unwrap();
        assert!(write_report(&report, &path).is_err());
        let restored: ConsolidatedValidation =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(report.report_sha256, restored.report_sha256);
    }

    #[test]
    fn contradictory_scoped_counts_flag_but_unlike_units_and_unknown_viewer_do_not_fake_zero() {
        let m = manifest();
        let scope = "e".repeat(64);
        let mut counts = vec![
            CountObservation {
                scope_sha256: scope.clone(),
                unit: CountUnit::SourceRows,
                count: Evidence::Publisher(32747),
                claim: CountClaim::ScopeTotal,
                completeness: CountCompleteness::Complete,
            },
            CountObservation {
                scope_sha256: scope,
                unit: CountUnit::SourceRows,
                count: Evidence::Publisher(14650),
                claim: CountClaim::ScopeTotal,
                completeness: CountCompleteness::Partial,
            },
        ];
        let r = consolidate(ConsolidationInput {
            counts: &counts,
            ..input(&m)
        })
        .unwrap();
        assert_eq!(r.count_discrepancies, vec![[0, 1]]);
        assert_eq!(
            r.checks[&Check::CountConsistency],
            Evidence::Measured(Finding::Fail)
        );
        counts[1].unit = CountUnit::Documents;
        assert_eq!(
            consolidate(ConsolidationInput {
                counts: &counts,
                ..input(&m)
            })
            .unwrap()
            .checks[&Check::CountConsistency],
            Evidence::Unknown
        );
        counts[1].unit = CountUnit::SourceRows;
        counts[1].count = Evidence::Unknown;
        let r = consolidate(ConsolidationInput {
            counts: &counts,
            ..input(&m)
        })
        .unwrap();
        assert_eq!(r.checks[&Check::CountConsistency], Evidence::Unknown);
        assert!(r.count_discrepancies.is_empty());
        assert_eq!(r.corpus_rows, Evidence::Unknown);
    }
    #[test]
    fn rendered_artifact_binding_changes_invalidate_consolidation_not_reselect() {
        use crate::prefix_validation::{self, RenderedRecord, RenderingIdentity};
        let mut m = manifest();
        let identity = RenderingIdentity {
            tokenizer_id: "synthetic".into(),
            tokenizer_revision: "a".repeat(40),
            tokenizer_sha256: "b".repeat(64),
            chat_template_sha256: "c".repeat(64),
            add_special_tokens: false,
            add_generation_prompt: true,
            hint_nonce_included: false,
            renderer_version: "synthetic.v1".into(),
        };
        let records = vec![
            RenderedRecord {
                record_id: "private-a".into(),
                group_id: None,
                tokens: vec![1, 2],
                rendered_body_sha256: "d".repeat(64),
                supplied_input_target: None,
            },
            RenderedRecord {
                record_id: "private-b".into(),
                group_id: None,
                tokens: vec![1, 3],
                rendered_body_sha256: "e".repeat(64),
                supplied_input_target: None,
            },
        ];
        let mut overlap =
            prefix_validation::validate(&records, &[], identity.clone(), 2, 4, None).unwrap();
        m.tokenizer_revision = Some(identity.tokenizer_revision);
        m.chat_template_sha256 = Some(identity.chat_template_sha256);
        m.checks.insert(
            "rendered_overlap".into(),
            Evidence::Measured(format!(
                "artifact_sha256={}; n=2",
                sha256(&serde_json::to_vec(&overlap).unwrap())
            )),
        );
        let report = consolidate(ConsolidationInput {
            overlap: Some(&overlap),
            ..input(&m)
        })
        .unwrap();
        assert_eq!(
            report.checks[&Check::RenderedSample],
            Evidence::Measured(Finding::Pass)
        );
        assert_eq!(report.rendered_sample_size, Some(2));
        assert_eq!(report.corpus_rows, Evidence::Unknown);
        overlap.identity.tokenizer_sha256 = "f".repeat(64);
        assert!(consolidate(ConsolidationInput {
            overlap: Some(&overlap),
            ..input(&m)
        })
        .is_err());
        let mut changed = m.clone();
        changed.adapter_version = "2".into();
        assert_ne!(
            consolidate(input(&changed)).unwrap().report_sha256,
            consolidate(input(&m)).unwrap().report_sha256
        );
    }

    #[test]
    fn unequal_partial_counts_are_not_full_scope_contradictions() {
        let m = manifest();
        let mut counts = vec![
            CountObservation {
                scope_sha256: "e".repeat(64),
                unit: CountUnit::SourceRows,
                count: Evidence::Publisher(8),
                claim: CountClaim::ObservedSubset,
                completeness: CountCompleteness::Partial,
            },
            CountObservation {
                scope_sha256: "e".repeat(64),
                unit: CountUnit::SourceRows,
                count: Evidence::Measured(80),
                claim: CountClaim::ScopeTotal,
                completeness: CountCompleteness::Complete,
            },
        ];
        let report = consolidate(ConsolidationInput {
            counts: &counts,
            ..input(&m)
        })
        .unwrap();
        assert!(
            report.count_discrepancies.is_empty(),
            "partial viewer rows cannot contradict a full-scope total"
        );
        assert_eq!(report.checks[&Check::CountConsistency], Evidence::Unknown);
        counts[0].completeness = CountCompleteness::Complete;
        counts[0].claim = CountClaim::ScopeTotal;
        let report = consolidate(ConsolidationInput {
            counts: &counts,
            ..input(&m)
        })
        .unwrap();
        assert_eq!(report.count_discrepancies.len(), 1);
        let schema: serde_json::Value = serde_json::from_str(include_str!(
            "../schemas/source-validation-consolidation.v1.json"
        ))
        .unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        let mut value = serde_json::to_value(&report).unwrap();
        assert!(validator.is_valid(&value));
        value["counts"][0].as_object_mut().unwrap().remove("claim");
        assert!(!validator.is_valid(&value));
        assert_eq!(
            report.checks[&Check::CountConsistency],
            Evidence::Measured(Finding::Fail)
        );
    }

    #[test]
    fn unavailable_counts_cannot_be_serialized_as_zero_and_partial_counts_remain_partial() {
        let m = manifest();
        let mut counts = vec![CountObservation {
            scope_sha256: "e".repeat(64),
            unit: CountUnit::SourceRows,
            count: Evidence::Publisher(0),
            claim: CountClaim::ScopeTotal,
            completeness: CountCompleteness::Unavailable,
        }];
        assert!(consolidate(ConsolidationInput {
            counts: &counts,
            ..input(&m)
        })
        .is_err());
        counts[0].count = Evidence::Unknown;
        assert_eq!(
            consolidate(ConsolidationInput {
                counts: &counts,
                ..input(&m)
            })
            .unwrap()
            .checks[&Check::CountConsistency],
            Evidence::Unknown
        );
        counts[0].count = Evidence::Publisher(8);
        counts[0].completeness = CountCompleteness::Partial;
        counts.push(counts[0].clone());
        assert_eq!(
            consolidate(ConsolidationInput {
                counts: &counts,
                ..input(&m)
            })
            .unwrap()
            .checks[&Check::CountConsistency],
            Evidence::Unknown
        );
        for count in &mut counts {
            count.completeness = CountCompleteness::Complete;
        }
        assert_eq!(
            consolidate(ConsolidationInput {
                counts: &counts,
                ..input(&m)
            })
            .unwrap()
            .checks[&Check::CountConsistency],
            Evidence::Measured(Finding::Pass)
        );
    }

    #[test]
    fn component_or_row_review_must_be_complete_bound_and_authorized() {
        let m = manifest();
        let review = ComponentReview::bind(&m, "private-component", m.rights.clone()).unwrap();
        let scopes = vec![review.scope_sha256.clone()];
        let mut reviews = vec![review];
        let r = consolidate(ConsolidationInput {
            required_component_scopes: &scopes,
            component_inventory_complete: true,
            component_reviews: &reviews,
            ..input(&m)
        })
        .unwrap();
        assert_eq!(
            r.checks[&Check::DeclaredComponentCoverage],
            Evidence::Measured(Finding::Pass)
        );
        let mut changed = m.clone();
        changed.selected_ids_sha256 = "f".repeat(64);
        assert!(consolidate(ConsolidationInput {
            required_component_scopes: &scopes,
            component_inventory_complete: true,
            component_reviews: &reviews,
            ..input(&changed)
        })
        .is_err());
        let mut declared = m.clone();
        declared
            .rights
            .component_terms
            .insert("private-component".into(), "source terms".into());
        assert_eq!(
            consolidate(ConsolidationInput {
                component_inventory_complete: true,
                ..input(&declared)
            })
            .unwrap()
            .checks[&Check::DeclaredComponentCoverage],
            Evidence::Unknown
        );
        reviews[0].policy.license = None;
        assert_eq!(
            consolidate(ConsolidationInput {
                required_component_scopes: &scopes,
                component_inventory_complete: true,
                component_reviews: &reviews,
                ..input(&m)
            })
            .unwrap()
            .checks[&Check::DeclaredComponentCoverage],
            Evidence::Unknown
        );
        reviews[0].scope_sha256 = "f".repeat(64);
        assert!(consolidate(ConsolidationInput {
            required_component_scopes: &scopes,
            component_inventory_complete: true,
            component_reviews: &reviews,
            ..input(&m)
        })
        .is_err());
    }
}
