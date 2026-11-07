// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Explicit constructed prompt candidates, never original-text or replay claims.
use crate::canonical_input::{CanonicalBody, CanonicalRecord, DatasetEnvelopeRef};
use crate::public_dataset::{
    sha256, Evidence, IngestionBody, IngestionRecord, ValidationManifest, SCHEMA_VERSION,
};
use crate::trace_text::{construct_jsonl, ConstructedTrace, TextConstructionConfig, TraceSource};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstructedCandidates {
    /// Private original lineage and newly constructed bodies remain separate.
    pub construction: ConstructedTrace,
    pub manifest: ValidationManifest,
    /// Candidates only: runtime owns selection, actual arrival and issuance.
    pub records: Vec<CanonicalRecord>,
}

/// Reverify source/rights and raw construction, then explicitly project new
/// literals as Prompt candidates using the frozen checked converter. No public
/// API trusts deserialized construction claims as fresh validation. Reference
/// source output lengths remain metadata, not silently requested output budgets.
/// Source graph/history/relative times stay private and are not flattened into
/// invented runtime causal/session/arrival semantics.
pub fn construct_candidates_jsonl(
    source: TraceSource<'_>,
    tokenizer: &Path,
    config: &TextConstructionConfig,
) -> Result<ConstructedCandidates> {
    let rights = source.rights.clone();
    let construction = construct_jsonl(source, tokenizer, config)?;
    let bodies: Vec<CanonicalBody> = construction
        .requests
        .iter()
        .map(|r| CanonicalBody::Prompt(r.text.clone()))
        .collect();
    let ids: Vec<&str> = construction
        .requests
        .iter()
        .map(|r| r.source_row_id.as_str())
        .collect();
    let manifest = ValidationManifest {
        schema_version:SCHEMA_VERSION.into(),origin:construction.source_trace.source.clone(),rights,
        adapter_id:"constructed-source-trace".into(),adapter_version:"1".into(),
        field_mapping:BTreeMap::from([
            ("source_row_id".into(),"preserved publisher ID or explicitly mapped source position".into()),
            ("body".into(),"new seeded literals; NOT recovered original source text".into()),
            ("source_output_length".into(),"reference metadata only; no requested output target".into()),
            ("source_parent_time".into(),"private source metadata; no inferred runtime request points".into()),
        ]),
        transformations:vec!["construct literal segments preserving verified known full-prefix graph; seeded constructed tails".into(),
            format!("tokenizer_sha256={}",config.tokenizer_sha256),
            format!("construction_sha256={}",construction.construction_sha256),
            format!("source_normalization_sha256={}",construction.source_trace.normalization_sha256)],
        tokenizer_revision:Some(config.tokenizer_revision.clone()),chat_template_sha256:None,
        generator_version:Some("trace-literals.v1".into()),corpus_rows:Evidence::Unknown,
        selected_source_rows:construction.requests.len() as u64,target_count:bodies.len() as u64,
        selected_ids_sha256:sha256(&serde_json::to_vec(&ids)?),body_order_sha256:sha256(&serde_json::to_vec(&bodies)?),
        checks:BTreeMap::from([
            ("constructed_raw_prompt_prefixes_and_counts".into(),Evidence::Measured("exact offline pinned-tokenizer checks; raw prompts only".into())),
            ("rendered_token_overlap".into(),Evidence::Unknown),
            ("physical_cache_evidence".into(),Evidence::Unknown),
            ("original_text_recovery".into(),Evidence::Unknown),
            ("runtime_arrival_and_dependency_replay".into(),Evidence::Unknown),
        ]),exclusions:vec![],limitations:vec![
            "Candidates contain constructed text, never original source recovery or semantic replay.".into(),
            "Source reference output lengths are not requested output budgets; no output narrowing or clipping.".into(),
            "Runtime owns selection, source graph interpretation, scheduling and final issued-body/token validation.".into(),
            "Corpus coverage and physical cache behavior remain unknown; no original partial-tail reuse claim.".into(),
        ],
    };
    let envelope = DatasetEnvelopeRef::from(manifest.envelope()?);
    let mut records = Vec::new();
    for (row, body) in construction.requests.iter().zip(bodies) {
        let record_id = format!(
            "constructed-{}",
            sha256(&serde_json::to_vec(&(
                &construction.source_trace.source_file_sha256,
                &row.source_row_id,
                &construction.construction_sha256
            ))?)
        );
        let CanonicalBody::Prompt(text) = body else {
            unreachable!("explicit constructed Prompt")
        };
        let record = IngestionRecord {
            record_id,
            session_id: None,
            trace_id: Some(format!("constructed-{}", construction.construction_sha256)),
            source_row_id: row.source_row_id.clone(),
            source_file_sha256: construction.source_trace.source_file_sha256.clone(),
            group_id: None,
            turn_index: None,
            output_budget: None,
            body: IngestionBody::Prompt { text },
        };
        record.validate(&construction.source_trace.source)?;
        records.push(record.into_canonical(envelope.clone())?);
    }
    Ok(ConstructedCandidates {
        construction,
        manifest,
        records,
    })
}
