// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Declarative document/question adapters. Answers and patches are never mapped.

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::public_dataset::{sha256, CanonicalBody, CanonicalRecord, DatasetOrigin, RightsPolicy};

/// Explicit mapping; source layout is not inferred from dataset name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DocumentMapping {
    pub adapter_id: String,
    pub adapter_version: String,
    pub expected_config: String,
    pub expected_split: String,
    pub row_id_field: String,
    pub document_id_field: String,
    pub document_field: String,
    pub questions_field: String,
    pub question_array: bool,
    pub output_budget: u64,
}

/// Derived target budget is separate from source byte/row budgets.
#[derive(Debug, Clone, Copy)]
pub struct AdapterLimits {
    pub max_targets: usize,
    pub max_body_bytes: usize,
}

fn string<'a>(row: &'a Value, field: &str) -> Result<&'a str> {
    let value = row
        .get(field)
        .and_then(Value::as_str)
        .with_context(|| format!("missing/non-string adapter field {field}"))?;
    ensure!(!value.trim().is_empty(), "empty adapter field {field}");
    Ok(value)
}

/// Map single questions or nested question arrays. Documents precede questions
/// at rendering time; no silent truncation/retrieval or answer-field copying.
pub fn documents(
    rows: &[Value],
    origin: &DatasetOrigin,
    source_file_sha256: &str,
    rights: &RightsPolicy,
    mapping: &DocumentMapping,
    limits: AdapterLimits,
) -> Result<Vec<CanonicalRecord>> {
    origin.validate()?;
    rights.authorize()?;
    ensure!(
        origin.config == mapping.expected_config && origin.split == mapping.expected_split,
        "adapter config/split mismatch"
    );
    ensure!(
        !mapping.adapter_id.is_empty() && !mapping.adapter_version.is_empty(),
        "adapter version required"
    );
    ensure!(
        origin
            .files
            .iter()
            .any(|f| f.sha256.eq_ignore_ascii_case(source_file_sha256)),
        "adapter file is not in origin"
    );
    for field in [&mapping.document_field, &mapping.questions_field] {
        ensure!(
            ![
                "answer",
                "answers",
                "output",
                "outputs",
                "patch",
                "test_patch",
                "solution"
            ]
            .contains(&field.as_str()),
            "gold answer/patch fields cannot be request mappings"
        );
    }
    let mut targets = Vec::new();
    let mut bytes = 0usize;
    let mut ids = std::collections::BTreeSet::new();
    for row in rows {
        let row_id = string(row, &mapping.row_id_field)?;
        let doc_id = string(row, &mapping.document_id_field)?;
        let document = string(row, &mapping.document_field)?;
        let questions: Vec<&str> = if mapping.question_array {
            let values = row
                .get(&mapping.questions_field)
                .and_then(Value::as_array)
                .context("question array required")?;
            ensure!(!values.is_empty(), "empty question array");
            ensure!(
                values.len() <= limits.max_targets.saturating_sub(targets.len()),
                "derived target count budget exceeded"
            );
            values
                .iter()
                .map(|value| value.as_str().context("question must be a string"))
                .collect::<Result<_>>()?
        } else {
            vec![string(row, &mapping.questions_field)?]
        };
        let group_id = sha256(&serde_json::to_vec(&(
            &origin.source_id,
            doc_id,
            sha256(document.as_bytes()),
        ))?);
        for (question_index, question) in questions.into_iter().enumerate() {
            ensure!(!question.trim().is_empty(), "empty question");
            ensure!(
                targets.len() < limits.max_targets,
                "derived target count budget exceeded"
            );
            bytes = bytes
                .checked_add(document.len())
                .and_then(|b| b.checked_add(question.len()))
                .context("derived body byte count overflow")?;
            ensure!(
                bytes <= limits.max_body_bytes,
                "derived body byte budget exceeded"
            );
            let record_id = sha256(&serde_json::to_vec(&(
                &origin.source_id,
                &origin.resolved_revision,
                source_file_sha256,
                row_id,
                question_index,
            ))?);
            ensure!(
                ids.insert(record_id.clone()),
                "duplicate source target identity"
            );
            let record = CanonicalRecord {
                record_id,
                session_id: None,
                trace_id: None,
                turn_index: None,
                source_row_id: row_id.into(),
                source_file_sha256: source_file_sha256.into(),
                group_id: Some(group_id.clone()),
                output_budget: Some(mapping.output_budget),
                body: CanonicalBody::DocumentQuestion {
                    document_id: doc_id.into(),
                    document: document.into(),
                    question: question.into(),
                },
            };
            record.validate(origin)?;
            targets.push(record);
        }
    }
    Ok(targets)
}

/// Deterministic document-first text. Chat framing is applied by the caller and
/// must then pass the pinned rendered-token validator. No clipping occurs here.
pub fn render_document(record: &CanonicalRecord) -> Result<String> {
    let CanonicalBody::DocumentQuestion {
        document, question, ..
    } = &record.body
    else {
        anyhow::bail!("document/question record required");
    };
    Ok(format!("{document}\n\nQuestion:\n{question}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::public_dataset::{AccessDecision, SourceFile};
    use serde_json::json;
    fn fixture() -> (DatasetOrigin, RightsPolicy, DocumentMapping, AdapterLimits) {
        let value: Value = serde_json::from_str(include_str!(
            "../test-data/public-datasets/loogle-excerpt.v1.json"
        ))
        .unwrap();
        let origin = serde_json::from_value(value["source"].clone()).unwrap();
        let rights = RightsPolicy { decision: AccessDecision::Allowed, license: Some("CC-BY-SA-4.0".into()), license_text_sha256: Some("a".repeat(64)), component_terms: std::collections::BTreeMap::new(), gated: false, terms_review: None, redistribution_allowed: true, privacy_handling: "Historical public biography excerpt; synthetic rights-policy fixture only, not a legal clearance".into(), unresolved: vec![] };
        let mapping = DocumentMapping {
            adapter_id: "loogle-document-question".into(),
            adapter_version: "1".into(),
            expected_config: "longdep_qa".into(),
            expected_split: "test".into(),
            row_id_field: "id".into(),
            document_id_field: "doc_id".into(),
            document_field: "context".into(),
            questions_field: "question".into(),
            question_array: false,
            output_budget: 16,
        };
        (
            origin,
            rights,
            mapping,
            AdapterLimits {
                max_targets: 8,
                max_body_bytes: 8192,
            },
        )
    }
    #[test]
    fn pinned_excerpt_mapping_is_document_first_and_answer_free() {
        let value: Value = serde_json::from_str(include_str!(
            "../test-data/public-datasets/loogle-excerpt.v1.json"
        ))
        .unwrap();
        let (origin, rights, mapping, limits) = fixture();
        let mut row = value["row"].clone();
        row["answer"] = json!("GOLD_SENTINEL_DO_NOT_SEND");
        let records = documents(
            &[row],
            &origin,
            &origin.files[0].sha256,
            &rights,
            &mapping,
            limits,
        )
        .unwrap();
        assert_eq!(records.len(), 1);
        let text = render_document(&records[0]).unwrap();
        assert!(text.starts_with(" Early life."));
        assert!(text.contains("Question:\nHow many"));
        assert!(!text.contains("GOLD_SENTINEL"));
        assert_eq!(records[0].output_budget, Some(16));
        let mut bad_mapping = mapping.clone();
        bad_mapping.document_field = "answer".into();
        assert!(documents(
            &[value["row"].clone()],
            &origin,
            &origin.files[0].sha256,
            &rights,
            &bad_mapping,
            limits
        )
        .is_err());
    }
    #[test]
    fn nested_questions_groups_and_identity() {
        let (origin, rights, mut mapping, limits) = fixture();
        mapping.question_array = true;
        let row = json!({"id":"row", "doc_id":"doc", "context":"shared", "question":["one", "two"], "outputs":["SECRET1", "SECRET2"]});
        let a = documents(
            &[row.clone()],
            &origin,
            &origin.files[0].sha256,
            &rights,
            &mapping,
            limits,
        )
        .unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].group_id, a[1].group_id);
        assert_ne!(a[0].record_id, a[1].record_id);
        assert_eq!(
            a,
            documents(
                &[row.clone()],
                &origin,
                &origin.files[0].sha256,
                &rights,
                &mapping,
                limits
            )
            .unwrap()
        );
        let mut changed = row.clone();
        changed["id"] = json!("row2");
        changed["context"] = json!("different");
        let b = documents(
            &[changed],
            &origin,
            &origin.files[0].sha256,
            &rights,
            &mapping,
            limits,
        )
        .unwrap();
        assert_ne!(a[0].group_id, b[0].group_id);
        assert!(documents(
            &[row.clone(), row],
            &origin,
            &origin.files[0].sha256,
            &rights,
            &mapping,
            limits
        )
        .is_err());
    }
    #[test]
    fn missing_empty_config_rights_and_budgets_fail() {
        let (mut origin, mut rights, mapping, mut limits) = fixture();
        let row = json!({"id":"row", "doc_id":"doc", "context":"é", "question":"q"});
        for field in ["id", "doc_id", "context", "question"] {
            let mut bad = row.clone();
            bad.as_object_mut().unwrap().remove(field);
            assert!(documents(
                &[bad],
                &origin,
                &origin.files[0].sha256,
                &rights,
                &mapping,
                limits
            )
            .is_err());
        }
        let mut bad = row.clone();
        bad["context"] = json!("");
        assert!(documents(
            &[bad],
            &origin,
            &origin.files[0].sha256,
            &rights,
            &mapping,
            limits
        )
        .is_err());
        origin.config = "wrong".into();
        assert!(documents(
            &[row.clone()],
            &origin,
            &origin.files[0].sha256,
            &rights,
            &mapping,
            limits
        )
        .is_err());
        origin.config = "longdep_qa".into();
        rights.decision = AccessDecision::Denied;
        assert!(documents(
            &[row.clone()],
            &origin,
            &origin.files[0].sha256,
            &rights,
            &mapping,
            limits
        )
        .is_err());
        rights.decision = AccessDecision::Allowed;
        limits.max_targets = 0;
        assert!(documents(
            &[row.clone()],
            &origin,
            &origin.files[0].sha256,
            &rights,
            &mapping,
            limits
        )
        .is_err());
        limits.max_targets = 8;
        limits.max_body_bytes = 2;
        assert!(documents(
            &[row],
            &origin,
            &origin.files[0].sha256,
            &rights,
            &mapping,
            limits
        )
        .is_err());
        // SourceFile remains an explicit digest/size contract, never inferred from an excerpt.
        let _: &SourceFile = &origin.files[0];
    }
}
