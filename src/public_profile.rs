// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Reproducible public document ingestion into the canonical runtime contract.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
#[cfg(feature = "tokenizer")]
use serde_json::json;
use serde_json::Value;

use crate::archive_inputs::{read_zip_members, ArchiveMember};
use crate::canonical_input::{CanonicalRecord, DatasetEnvelopeRef, Message, Role};
use crate::dataset_formats::{read_rows, DatasetFormat};
use crate::prefix_validation::{self, OverlapValidation, RenderingIdentity, SelectionConstraints};
use crate::public_adapters::{documents, render_document, AdapterLimits, DocumentMapping};
use crate::public_dataset::{
    sha256, DatasetOrigin, Evidence, IngestionLimits, RightsPolicy, ValidationManifest,
    SCHEMA_VERSION,
};

/// Explicit per-file parsing; no executable format sniffing or recursive archives.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "format", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputFormat {
    JsonArray,
    Jsonl,
    Parquet,
    Zip {
        members: Vec<ArchiveMember>,
        max_entries: usize,
    },
}

/// Both source and derived-target budgets must be provided by the operator.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileLimits {
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    pub max_rows: usize,
    pub max_targets: usize,
    pub max_body_bytes: usize,
    pub max_pairs: usize,
}

/// Local artifacts only. Paths never enter the public validation manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileRendering {
    pub identity: RenderingIdentity,
    pub tokenizer_path: PathBuf,
    pub template_path: PathBuf,
    pub special_tokens: Value,
    pub max_artifact_bytes: u64,
    pub max_rendered_bytes: usize,
    pub max_tokens: usize,
    pub template_fuel: u64,
}

/// A versioned declarative document job; no dataset scripts or auto-agreements.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicDatasetJob {
    pub schema_version: String,
    pub origin: DatasetOrigin,
    pub rights: RightsPolicy,
    pub mapping: DocumentMapping,
    pub formats: BTreeMap<String, InputFormat>,
    /// Empty means pinned Hub/cache access. Nonempty must cover every source file.
    pub local_files: BTreeMap<String, PathBuf>,
    pub limits: ProfileLimits,
    pub count: Option<usize>,
    pub constraints: SelectionConstraints,
    pub block_size_tokens: usize,
    pub rendering: Option<ProfileRendering>,
}

/// Metadata-only evidence. Canonical inputs contain payload and must stay private
/// unless their own redistribution/privacy policy explicitly permits publication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicProfileReport {
    pub manifest: ValidationManifest,
    pub overlap: Option<OverlapValidation>,
    pub human_summary: String,
}

/// Parse a local job under a fixed configuration-byte budget, without fetching URLs.
pub fn load_job(path: &Path) -> Result<PublicDatasetJob> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(262_145)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 262_144, "dataset job byte budget exceeded");
    serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("invalid public dataset job schema"))
}

/// Build inputs and a manifest before traffic. Unknown token validation remains
/// unknown. Any later runtime body/history mutation requires fresh token validation.
pub fn build(
    job: &PublicDatasetJob,
    cache: &Path,
    offline: bool,
) -> Result<(Vec<CanonicalRecord>, PublicProfileReport)> {
    ensure!(
        job.schema_version == "public-dataset-job.v1",
        "unsupported public dataset job version"
    );
    job.origin.validate()?;
    job.rights.authorize()?;
    ensure!(
        job.formats.len() == job.origin.files.len()
            && job
                .origin
                .files
                .iter()
                .all(|f| job.formats.contains_key(&f.path)),
        "explicit format required for every pinned file"
    );
    let constrained = job.constraints.content_unique || job.constraints.max_pair_lcp.is_some();
    ensure!(
        !constrained || job.rendering.is_some(),
        "rendered-token constraints require pinned rendering artifacts"
    );
    let limits = IngestionLimits {
        max_file_bytes: job.limits.max_file_bytes,
        max_total_bytes: job.limits.max_total_bytes,
        max_rows: job.limits.max_rows,
    };
    let paths = if job.local_files.is_empty() {
        crate::public_dataset::materialize(&job.origin, &job.rights, cache, offline, limits)?
    } else {
        ensure!(
            job.local_files.len() == job.origin.files.len(),
            "local source list must cover every pinned file"
        );
        let total = job.origin.files.iter().try_fold(0u64, |sum, f| {
            sum.checked_add(f.bytes)
                .context("source byte count overflow")
        })?;
        ensure!(
            total <= limits.max_total_bytes,
            "aggregate source byte budget exceeded"
        );
        job.origin
            .files
            .iter()
            .map(|f| {
                job.local_files
                    .get(&f.path)
                    .cloned()
                    .context("missing explicit local source file")
            })
            .collect::<Result<Vec<_>>>()?
    };
    let mut source_rows = 0usize;
    let mut body_bytes = 0usize;
    let mut adapted = Vec::new();
    let mut transforms = vec![
        "document-first-question-last.v1".into(),
        "selection:first-fit.v1".into(),
    ];
    for (file, path) in job.origin.files.iter().zip(&paths) {
        let remaining = IngestionLimits {
            max_rows: limits.max_rows.saturating_sub(source_rows),
            ..limits
        };
        let groups = match job.formats.get(&file.path).context("missing file format")? {
            InputFormat::JsonArray => vec![(
                None,
                read_rows(path, DatasetFormat::JsonArray, file, remaining)?,
            )],
            InputFormat::Jsonl => vec![(
                None,
                read_rows(path, DatasetFormat::Jsonl, file, remaining)?,
            )],
            InputFormat::Parquet => vec![(
                None,
                read_rows(path, DatasetFormat::Parquet, file, remaining)?,
            )],
            InputFormat::Zip {
                members,
                max_entries,
            } => {
                transforms.push(format!(
                    "archive-member-id-namespace.v1:{}",
                    serde_json::to_string(members)?
                ));
                read_zip_members(path, file, members, remaining, *max_entries)?
                    .into_iter()
                    .map(|g| (Some(g.member.path), g.rows))
                    .collect()
            }
        };
        for (member, mut rows) in groups {
            source_rows = source_rows
                .checked_add(rows.len())
                .context("source row count overflow")?;
            ensure!(
                source_rows <= limits.max_rows,
                "aggregate source row budget exceeded"
            );
            if let Some(member) = member {
                for row in &mut rows {
                    let id = row
                        .get(&job.mapping.row_id_field)
                        .and_then(Value::as_str)
                        .context("archive row identity required")?;
                    let id = format!("{member}#{id}");
                    row[&job.mapping.row_id_field] = Value::String(id);
                }
            }
            let records = documents(
                &rows,
                &job.origin,
                &file.sha256,
                &job.rights,
                &job.mapping,
                AdapterLimits {
                    max_targets: job.limits.max_targets.saturating_sub(adapted.len()),
                    max_body_bytes: job.limits.max_body_bytes.saturating_sub(body_bytes),
                },
            )?;
            for record in &records {
                body_bytes = body_bytes
                    .checked_add(render_document(record)?.len())
                    .context("derived body byte count overflow")?;
                ensure!(
                    body_bytes <= job.limits.max_body_bytes,
                    "aggregate derived body byte budget exceeded"
                );
            }
            adapted.extend(records);
        }
    }
    ensure!(!adapted.is_empty(), "empty public dataset profile");
    let mut ids = BTreeSet::new();
    ensure!(
        adapted.iter().all(|r| ids.insert(r.record_id.clone())),
        "duplicate derived target identity"
    );
    let count = job.count.unwrap_or(adapted.len());
    ensure!(
        count > 0 && count <= adapted.len(),
        "requested target count cannot be achieved"
    );
    #[cfg(feature = "tokenizer")]
    let mut rendered = Vec::new();
    #[cfg(not(feature = "tokenizer"))]
    let rendered = Vec::new();
    if let Some(rendering) = &job.rendering {
        #[cfg(not(feature = "tokenizer"))]
        {
            let _ = rendering;
            anyhow::bail!("pinned public profile rendering requires --features tokenizer");
        }
        #[cfg(feature = "tokenizer")]
        {
            ensure!(
                count
                    .checked_mul(adapted.len())
                    .is_some_and(|n| n <= job.limits.max_pairs),
                "rendered selection work budget exceeded"
            );
            for record in &adapted {
                let body = serde_json::to_vec(
                    &json!({"messages":[{"role":"user","content":render_document(record)?}], "max_tokens":record.output_budget}),
                )?;
                let result = crate::chat_rendering::render_wire_body(
                    &body,
                    &rendering.tokenizer_path,
                    &rendering.template_path,
                    &rendering.identity,
                    &rendering.special_tokens,
                    crate::chat_rendering::RenderLimits {
                        max_artifact_bytes: rendering.max_artifact_bytes,
                        max_body_bytes: job.limits.max_body_bytes,
                        max_rendered_bytes: rendering.max_rendered_bytes,
                        max_tokens: rendering.max_tokens,
                        template_fuel: rendering.template_fuel,
                    },
                )?;
                rendered.push(prefix_validation::RenderedRecord {
                    record_id: record.record_id.clone(),
                    group_id: record.group_id.clone(),
                    tokens: result.token_ids,
                    rendered_body_sha256: result.rendered_text_sha256,
                    supplied_input_target: None,
                });
            }
        }
    }
    let overlap = if let Some(rendering) = &job.rendering {
        let chosen = prefix_validation::select(&rendered, count, job.constraints)?;
        let selected_ids: BTreeSet<_> = chosen.iter().map(|r| r.record_id.as_str()).collect();
        adapted.retain(|r| selected_ids.contains(r.record_id.as_str()));
        Some(prefix_validation::validate(
            &chosen,
            &[],
            rendering.identity.clone(),
            job.block_size_tokens,
            job.limits.max_pairs,
            None,
        )?)
    } else {
        adapted.truncate(count);
        None
    };
    let selected_rows: BTreeSet<_> = adapted
        .iter()
        .map(|r| (&r.source_file_sha256, &r.source_row_id))
        .collect();
    let body_hashes = adapted
        .iter()
        .map(|r| render_document(r).map(|s| sha256(s.as_bytes())))
        .collect::<Result<Vec<_>>>()?;
    let mut checks = BTreeMap::from([("selected_files_source_rows".into(), Evidence::Measured(source_rows.to_string())), ("answer_patch_field_exclusion".into(), Evidence::Measured("Only explicit document/question mapping fields entered requests; semantic leakage remains unmeasured".into()))]);
    checks.insert(
        "rendered_overlap".into(),
        match &overlap {
            Some(report) => Evidence::Measured(format!(
                "artifact_sha256={}; n={}",
                sha256(&serde_json::to_vec(report)?),
                report.sample_size
            )),
            None => Evidence::Unknown,
        },
    );
    let manifest = ValidationManifest {
        schema_version: SCHEMA_VERSION.into(), origin: job.origin.clone(), rights: job.rights.clone(), adapter_id: job.mapping.adapter_id.clone(), adapter_version: job.mapping.adapter_version.clone(),
        field_mapping: BTreeMap::from([("source_row_id".into(), job.mapping.row_id_field.clone()), ("document_id".into(), job.mapping.document_id_field.clone()), ("document".into(), job.mapping.document_field.clone()), ("question".into(), job.mapping.questions_field.clone())]),
        transformations: transforms, tokenizer_revision: job.rendering.as_ref().map(|r| r.identity.tokenizer_revision.clone()), chat_template_sha256: job.rendering.as_ref().map(|r| r.identity.chat_template_sha256.clone()), generator_version: None,
        corpus_rows: Evidence::Unknown, selected_source_rows: selected_rows.len() as u64, target_count: adapted.len() as u64,
        selected_ids_sha256: sha256(&serde_json::to_vec(&adapted.iter().map(|r| &r.record_id).collect::<Vec<_>>())?), body_order_sha256: sha256(&serde_json::to_vec(&body_hashes)?), checks,
        exclusions: vec!["Source answers, outputs, patches and unrelated metadata omitted from request mapping".into()],
        limitations: vec!["Selected-file recount is not a corpus certificate; first-fit selection is a convenience sample".into(), "Profile text fingerprints are not final runtime HTTP body fingerprints; rerender after transformations".into(), "Rights policy is operator-recorded, not independent legal/privacy clearance".into()],
    };
    let source: DatasetEnvelopeRef = manifest.envelope()?.into();
    let records = adapted
        .into_iter()
        .map(|record| {
            let input = CanonicalRecord {
                record_id: record.record_id.clone(),
                session_id: record.session_id.clone(),
                turn_index: record
                    .turn_index
                    .map(u32::try_from)
                    .transpose()
                    .context("runtime turn index overflow")?,
                group_id: record.group_id.clone(),
                trace_id: record.trace_id.clone(),
                arrival_offset_ms: None,
                messages: vec![Message {
                    role: Role::User,
                    content: Some(render_document(&record)?),
                    tool_calls: vec![],
                    tool_call_id: None,
                }],
                expected_output_tokens: record
                    .output_budget
                    .map(u32::try_from)
                    .transpose()
                    .context("runtime output target overflow")?,
                source: source.clone(),
            };
            input.validate()?;
            Ok(input)
        })
        .collect::<Result<Vec<_>>>()?;
    let human_summary = manifest.summary();
    Ok((
        records,
        PublicProfileReport {
            manifest,
            overlap,
            human_summary,
        },
    ))
}

/// Write local payload inputs and metadata-only report only after successful build.
/// Output files are separate; callers must not publish payload by default.
pub fn write_profile(
    records: &[CanonicalRecord],
    report: &PublicProfileReport,
    output: &Path,
    metadata: &Path,
) -> Result<()> {
    use std::io::Write;
    ensure!(output != metadata, "input and metadata outputs must differ");
    let mut data = tempfile::NamedTempFile::new_in(
        output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new(".")),
    )?;
    let mut evidence = tempfile::NamedTempFile::new_in(
        metadata
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new(".")),
    )?;
    for record in records {
        serde_json::to_writer(&mut data, record)?;
        data.write_all(b"\n")?;
    }
    serde_json::to_writer_pretty(&mut evidence, report)?;
    data.flush()?;
    evidence.flush()?;
    data.persist_noclobber(output).map_err(|e| e.error)?;
    evidence.persist_noclobber(metadata).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn offline_job_exports_runtime_inputs_and_unknown_token_status() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("source.jsonl");
        let bytes = b"{\"id\":\"1\",\"doc_id\":\"d\",\"context\":\"shared\",\"question\":\"one\",\"answer\":\"GOLD\"}\n{\"id\":\"2\",\"doc_id\":\"d\",\"context\":\"shared\",\"question\":\"two\"}\n";
        std::fs::write(&file, bytes).unwrap();
        let job: PublicDatasetJob = serde_json::from_value(json!({
            "schema_version":"public-dataset-job.v1", "origin":{"source_id":"example/fixture","resolved_revision":"a".repeat(40),"config":"default","split":"test","files":[{"path":"source.jsonl","sha256":sha256(bytes),"bytes":bytes.len()}]},
            "rights":{"decision":"allowed","license":"Apache-2.0","license_text_sha256":"b".repeat(64),"component_terms":{},"gated":false,"terms_review":null,"redistribution_allowed":false,"privacy_handling":"synthetic fixture only","unresolved":[]},
            "mapping":{"adapter_id":"fixture","adapter_version":"1","expected_config":"default","expected_split":"test","row_id_field":"id","document_id_field":"doc_id","document_field":"context","questions_field":"question","question_array":false,"output_budget":8},
            "formats":{"source.jsonl":{"format":"jsonl"}},"local_files":{"source.jsonl":file},
            "limits":{"max_file_bytes":16384,"max_total_bytes":16384,"max_rows":8,"max_targets":8,"max_body_bytes":16384,"max_pairs":64},
            "count":null,"constraints":{"content_unique":false,"max_pair_lcp":null},"block_size_tokens":4,"rendering":null
        })).unwrap();
        let cache = dir.path().join("unused-cache");
        let (records, report) = build(&job, &cache, true).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].group_id, records[1].group_id);
        assert!(!records[0].messages[0]
            .content
            .as_ref()
            .unwrap()
            .contains("GOLD"));
        assert_eq!(
            report.manifest.checks["rendered_overlap"],
            Evidence::Unknown
        );
        assert_eq!(report.manifest.corpus_rows, Evidence::Unknown);
        assert!(!cache.exists());
        assert_eq!(report.manifest.selected_source_rows, 2);
        let output = dir.path().join("canonical.jsonl");
        let metadata = dir.path().join("validation.json");
        write_profile(&records, &report, &output, &metadata).unwrap();
        assert_eq!(
            crate::canonical_input::load_canonical_jsonl(
                &output,
                IngestionLimits {
                    max_file_bytes: 65536,
                    max_total_bytes: 65536,
                    max_rows: 8
                }
            )
            .unwrap(),
            records
        );
        let mut denied = job.clone();
        denied.rights.license = None;
        assert!(build(&denied, &cache, true).is_err());
        let mut constrained = job;
        constrained.constraints.content_unique = true;
        assert!(build(&constrained, &cache, true).is_err());
    }
}
