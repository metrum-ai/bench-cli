// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Optional pinned discovery metadata, never an automatic rights/access grant.
use crate::canonical_input::CanonicalRecord;
use crate::public_adapters::DocumentMapping;
use crate::public_dataset::SourceFile;
use crate::public_dataset::{Evidence, RightsPolicy};
use crate::public_profile::{self, PublicDatasetJob, PublicProfileReport};
use crate::validation_consolidation::{
    self, Check, ComponentReview, ConsolidatedValidation, ConsolidationInput, CountObservation,
    Finding,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogSupport {
    DocumentMapping,
    ReviewOnly,
    ExtractionRequired,
}
#[derive(Debug, Clone, Serialize)]
pub struct CatalogEntry {
    pub key: &'static str,
    pub source_id: &'static str,
    pub revision: &'static str,
    pub support: CatalogSupport,
    /// Card metadata only, not underlying content rights or legal clearance.
    pub card_license_hint: Option<&'static str>,
    pub caveats: &'static str,
    /// LFS metadata pin, not proof that this process downloaded/recounted it.
    pub file: Option<SourceFile>,
}
pub fn entry(key: &str) -> Result<CatalogEntry> {
    entries()
        .into_iter()
        .find(|e| e.key == key)
        .context("unknown optional catalog key")
}

/// Mapping support is NOT source/underlying-component license clearance.
pub fn document_mapping(key: &str, output_budget: u64) -> Result<DocumentMapping> {
    ensure!(
        key == "loogle-longdep-qa",
        "catalog source needs reviewed mapping/extraction before use"
    );
    Ok(DocumentMapping {
        adapter_id: "catalog-loogle-document-question".into(),
        adapter_version: "1".into(),
        expected_config: "longdep_qa".into(),
        expected_split: "test".into(),
        row_id_field: "id".into(),
        document_id_field: "doc_id".into(),
        document_field: "context".into(),
        questions_field: "question".into(),
        question_array: false,
        output_budget,
    })
}

pub struct CatalogReview<'a> {
    /// Private labels; a complete operator inventory is explicit, not inferred.
    pub components: &'a BTreeMap<String, RightsPolicy>,
    pub inventory_complete: bool,
    pub counts: &'a [CountObservation],
    pub origin_sha256: &'a str,
    pub selected_ids_sha256: &'a str,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogReviewJob {
    pub schema_version: String,
    pub origin_sha256: String,
    pub selected_ids_sha256: String,
    pub inventory_complete: bool,
    pub components: BTreeMap<String, RightsPolicy>,
    pub counts: Vec<CountObservation>,
}
impl CatalogReviewJob {
    pub fn review(&self) -> CatalogReview<'_> {
        CatalogReview {
            components: &self.components,
            inventory_complete: self.inventory_complete,
            counts: &self.counts,
            origin_sha256: &self.origin_sha256,
            selected_ids_sha256: &self.selected_ids_sha256,
        }
    }
}
fn binding_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn load_review(path: &Path) -> Result<CatalogReviewJob> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(262145)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 262144, "catalog review byte limit");
    let job: CatalogReviewJob = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("invalid catalog review schema"))?;
    ensure!(
        job.schema_version == "public-catalog-review.v1",
        "unsupported catalog review version"
    );
    ensure!(
        job.components.len() <= 10000 && job.counts.len() <= 1000,
        "catalog review metadata limit"
    );
    ensure!(
        binding_digest(&job.origin_sha256) && binding_digest(&job.selected_ids_sha256),
        "invalid catalog review binding digest"
    );
    Ok(job)
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogProfile {
    pub records: Vec<CanonicalRecord>,
    pub profile: PublicProfileReport,
    pub consolidation: ConsolidatedValidation,
}
/// Explicit source job and policy are still required. Review-only entries never
/// auto-map. This performs real bounded ingestion/rendering before eligibility;
/// it does not trust a deserialized consolidation report as fresh validation.
pub fn build(
    key: &str,
    job: &PublicDatasetJob,
    review: CatalogReview<'_>,
    cache: &Path,
    offline: bool,
) -> Result<CatalogProfile> {
    build_entry(&entry(key)?, job, review, cache, offline)
}
fn build_entry(
    entry: &CatalogEntry,
    job: &PublicDatasetJob,
    review: CatalogReview<'_>,
    cache: &Path,
    offline: bool,
) -> Result<CatalogProfile> {
    ensure!(
        matches!(entry.support, CatalogSupport::DocumentMapping),
        "catalog source is review/extraction-only"
    );
    ensure!(
        job.origin.source_id == entry.source_id && job.origin.resolved_revision == entry.revision,
        "catalog source/revision mismatch"
    );
    ensure!(
        entry
            .file
            .as_ref()
            .is_some_and(|file| job.origin.files == vec![file.clone()]),
        "catalog exact path/bytes/checksum mismatch"
    );
    ensure!(
        review.origin_sha256 == crate::public_dataset::sha256(&serde_json::to_vec(&job.origin)?),
        "catalog review source binding mismatch"
    );
    ensure!(
        binding_digest(review.selected_ids_sha256),
        "invalid catalog selection review digest"
    );
    let mapping = document_mapping(entry.key, job.mapping.output_budget)?;
    ensure!(
        serde_json::to_value(&mapping)? == serde_json::to_value(&job.mapping)?,
        "catalog mapping differs from reviewed profile"
    );
    ensure!(
        job.rendering.is_some(),
        "catalog profile requires pinned rendered-token validation"
    );
    ensure!(
        review.inventory_complete,
        "catalog component inventory requires explicit review"
    );
    job.rights.authorize()?;
    for policy in review.components.values() {
        policy.authorize()?;
    }
    ensure!(
        job.rights
            .component_terms
            .keys()
            .all(|key| review.components.contains_key(key)),
        "catalog component terms missing review"
    );
    let (records, profile) = public_profile::build(job, cache, offline)?;
    ensure!(
        review.selected_ids_sha256 == profile.manifest.selected_ids_sha256,
        "catalog review selection binding mismatch"
    );
    let reviews = review
        .components
        .iter()
        .map(|(key, policy)| ComponentReview::bind(&profile.manifest, key, policy.clone()))
        .collect::<Result<Vec<_>>>()?;
    let scopes = reviews
        .iter()
        .map(|r| r.scope_sha256.clone())
        .collect::<Vec<_>>();
    let consolidation = validation_consolidation::consolidate(ConsolidationInput {
        manifest: &profile.manifest,
        overlap: profile.overlap.as_ref(),
        counts: review.counts,
        required_component_scopes: &scopes,
        component_inventory_complete: true,
        component_reviews: &reviews,
    })?;
    for check in [
        Check::DeclaredAccessPolicy,
        Check::DeclaredComponentCoverage,
        Check::RenderedSample,
    ] {
        ensure!(
            consolidation.checks.get(&check) == Some(&Evidence::Measured(Finding::Pass)),
            "catalog required validation remains unknown/failed"
        );
    }
    ensure!(
        consolidation.checks.get(&Check::CountConsistency)
            != Some(&Evidence::Measured(Finding::Fail)),
        "catalog scoped count contradiction unresolved"
    );
    Ok(CatalogProfile {
        records,
        profile,
        consolidation,
    })
}

// Static discovery inventory is deliberately separate from profile eligibility.
pub fn entries() -> Vec<CatalogEntry> {
    vec![
        CatalogEntry { key:"loogle-longdep-qa",source_id:"bigai-nlco/LooGLE",revision:"4b50b4cb9333b48b3f7ddfd307c1d13369be5d1e",support:CatalogSupport::DocumentMapping,card_license_hint:Some("CC-BY-SA-4.0"),caveats:"Source/underlying components require recorded review. Retain attribution/share-alike. Committed excerpt is not full corpus verification.",file:Some(SourceFile { path:"data/longdep_qa.jsonl".into(),sha256:"6c047bc6e904e5fd632e44a2f2624d75e85fe0f871c6b74bcfffa918bf2f3624".into(),bytes:134_979_019 }) },
        CatalogEntry { key:"quality",source_id:"tasksource/QuALITY",revision:"4892b065e6a40f44fa61bc406df4c35fe91e9290",support:CatalogSupport::ReviewOnly,card_license_hint:None,caveats:"Per-article rights/jurisdiction must be retained; repeated article rows are not independent documents.",file:None },
        CatalogEntry { key:"narrativeqa",source_id:"deepmind/narrativeqa",revision:"2e643e7363944af1c33a652d1c87320d0871c4e4",support:CatalogSupport::ReviewOnly,card_license_hint:Some("Apache-2.0"),caveats:"Story/script component rights and source-file recount required; conflicting card/viewer counts are not corpus certification.",file:None },
        CatalogEntry { key:"longbench-v1",source_id:"zai-org/LongBench",revision:"5e628be450b7e67fb7ae6e201bd6d8f7056f7672",support:CatalogSupport::ReviewOnly,card_license_hint:None,caveats:"English words/Chinese characters are not tokens. Component rights unresolved; upstream code license is not blanket dataset clearance.",file:None },
        CatalogEntry { key:"longbench-v2",source_id:"zai-org/LongBench-v2",revision:"2b48e494f2c7a2f0af81aae178e05c7e1dde0fe9",support:CatalogSupport::ReviewOnly,card_license_hint:Some("Apache-2.0"),caveats:"Extreme contexts require target-model window checks; tokenizer success is not serving feasibility. Component review remains mandatory.",file:None },
        CatalogEntry { key:"infinitebench",source_id:"xinrongzhang2022/InfiniteBench",revision:"90f0394333616266d9fe85824ceaf505093cbaa5",support:CatalogSupport::ReviewOnly,card_license_hint:None,caveats:"Books/code/synthetic components need review; incomplete or empty viewer metadata is not zero corpus size.",file:None },
        CatalogEntry { key:"leval",source_id:"L4NLP/LEval",revision:"43b9dbf06c239360b183c76cb5d90ebe4f5e3070",support:CatalogSupport::ReviewOnly,card_license_hint:Some("GPL-3.0"),caveats:"Document and instruction-target counting units differ; nested mapping and underlying rights require validation.",file:None },
        CatalogEntry { key:"swe-bench-verified",source_id:"SWE-bench/SWE-bench_Verified",revision:"78f471bf655a3137b2e8a75af1501690ec009ec3",support:CatalogSupport::ExtractionRequired,card_license_hint:None,caveats:"Issue/repo/base-commit tasks are not full contexts or trajectories. Version/hash repository extraction; retain target repo licenses and exclude gold patch/test leakage. Never execute source scripts.",file:None },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inventory_never_auto_grants_rights_or_maps_code_tasks() {
        let entries = entries();
        assert_eq!(entries.len(), 8);
        for source in &entries {
            assert_eq!(source.revision.len(), 40);
            assert!(source.revision.bytes().all(|b| b.is_ascii_hexdigit()));
        }
        assert!(entry("unknown").is_err());
        assert!(document_mapping("swe-bench-verified", 8).is_err());
        assert!(document_mapping("quality", 8).is_err());
        assert_eq!(
            document_mapping("loogle-longdep-qa", 8)
                .unwrap()
                .document_field,
            "context"
        );
        assert_eq!(
            entry("loogle-longdep-qa").unwrap().file.unwrap().bytes,
            134_979_019
        );
    }
    #[test]
    fn review_loader_is_bounded_explicit_and_does_not_grant_inventory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("review.json");
        let mut value = serde_json::json!({"schema_version":"public-catalog-review.v1","origin_sha256":"a".repeat(64),"selected_ids_sha256":"b".repeat(64),"inventory_complete":false,"components":{},"counts":[]});
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(!load_review(&path).unwrap().review().inventory_complete);
        value["schema_version"] = serde_json::json!("unsupported");
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(load_review(&path).is_err());
        value["schema_version"] = serde_json::json!("public-catalog-review.v1");
        value["unknown_field"] = serde_json::json!(true);
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(load_review(&path).is_err());
        value.as_object_mut().unwrap().remove("unknown_field");
        value["selected_ids_sha256"] = serde_json::json!("bad-digest");
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(load_review(&path).is_err());
        std::fs::write(&path, vec![b' '; 262145]).unwrap();
        assert!(load_review(&path).is_err());
    }

    #[cfg(feature = "tokenizer")]
    #[test]
    fn catalog_pipeline_requires_rendering_rights_and_fresh_bound_consolidation() {
        use crate::public_dataset::sha256;
        let dir = tempfile::tempdir().unwrap();
        let bytes=b"{\"id\":\"private-a\",\"doc_id\":\"d\",\"context\":\"shared\",\"question\":\"one\",\"answer\":\"GOLD\"}\n{\"id\":\"private-b\",\"doc_id\":\"d\",\"context\":\"shared\",\"question\":\"two\"}\n";
        let source = dir.path().join("source.jsonl");
        std::fs::write(&source, bytes).unwrap();
        let mut job:PublicDatasetJob=serde_json::from_value(serde_json::json!({
            "schema_version":"public-dataset-job.v1","origin":{"source_id":"example/synthetic","resolved_revision":"a".repeat(40),"config":"longdep_qa","split":"test","files":[{"path":"source.jsonl","sha256":sha256(bytes),"bytes":bytes.len()}]},
            "rights":{"decision":"allowed","license":"Apache-2.0","license_text_sha256":"b".repeat(64),"component_terms":{},"gated":false,"terms_review":null,"redistribution_allowed":false,"privacy_handling":"synthetic only","unresolved":[]},
            "mapping":document_mapping("loogle-longdep-qa",8).unwrap(),"formats":{"source.jsonl":{"format":"jsonl"}},"local_files":{"source.jsonl":source},
            "limits":{"max_file_bytes":16384,"max_total_bytes":16384,"max_rows":8,"max_targets":8,"max_body_bytes":16384,"max_pairs":64},"count":null,"constraints":{"content_unique":false,"max_pair_lcp":null},"block_size_tokens":2,"rendering":null
        })).unwrap();
        let entry = CatalogEntry {
            key: "loogle-longdep-qa",
            source_id: "example/synthetic",
            revision: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            support: CatalogSupport::DocumentMapping,
            card_license_hint: None,
            caveats: "synthetic test only",
            file: Some(job.origin.files[0].clone()),
        };
        let components = BTreeMap::new();
        let cache = dir.path().join("unused-cache");
        let origin_binding = sha256(&serde_json::to_vec(&job.origin).unwrap());
        let placeholder_selection = "c".repeat(64);
        let review = || CatalogReview {
            components: &components,
            inventory_complete: true,
            counts: &[],
            origin_sha256: &origin_binding,
            selected_ids_sha256: &placeholder_selection,
        };
        assert!(build_entry(&entry, &job, review(), &cache, true).is_err());
        assert!(!cache.exists());
        let tokenizer=br#"{"version":"1.0","truncation":null,"padding":null,"added_tokens":[],"normalizer":null,"pre_tokenizer":{"type":"WhitespaceSplit"},"post_processor":null,"decoder":null,"model":{"type":"WordLevel","vocab":{"[UNK]":0,"shared":1,"Question:":2,"one":3,"two":4},"unk_token":"[UNK]"}}"#;
        let template = b"{% for message in messages %}{{ message.content }}{% endfor %}";
        let tok = dir.path().join("tokenizer.json");
        let tpl = dir.path().join("template.jinja");
        std::fs::write(&tok, tokenizer).unwrap();
        std::fs::write(&tpl, template).unwrap();
        job.rendering=Some(serde_json::from_value(serde_json::json!({"identity":{"tokenizer_id":"synthetic","tokenizer_revision":"a".repeat(40),"tokenizer_sha256":sha256(tokenizer),"chat_template_sha256":sha256(template),"add_special_tokens":false,"add_generation_prompt":false,"hint_nonce_included":false,"renderer_version":crate::chat_rendering::RENDERER_VERSION},"tokenizer_path":tok,"template_path":tpl,"special_tokens":{},"max_artifact_bytes":16384,"max_rendered_bytes":16384,"max_tokens":128,"template_fuel":10000})).unwrap());
        let (_, baseline_profile) = public_profile::build(&job, &cache, true).unwrap();
        let expected_selection = baseline_profile.manifest.selected_ids_sha256;
        let review = || CatalogReview {
            components: &components,
            inventory_complete: true,
            counts: &[],
            origin_sha256: &origin_binding,
            selected_ids_sha256: &expected_selection,
        };
        let result = build_entry(&entry, &job, review(), &cache, true).unwrap();
        let stale = CatalogReview {
            selected_ids_sha256: &placeholder_selection,
            ..review()
        };
        assert!(build_entry(&entry, &job, stale, &cache, true).is_err());
        assert_eq!(result.records.len(), 2);
        assert!(serde_json::to_string(&result.records)
            .unwrap()
            .find("GOLD")
            .is_none());
        assert_eq!(
            result.consolidation.checks[&Check::RenderedSample],
            Evidence::Measured(Finding::Pass)
        );
        assert_eq!(result.consolidation.corpus_rows, Evidence::Unknown);
        job.rights.decision = crate::public_dataset::AccessDecision::ReviewRequired;
        assert!(build_entry(&entry, &job, review(), &cache, true).is_err());
    }
}
