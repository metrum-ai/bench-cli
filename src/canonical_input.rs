// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Temporary consumption boundary for the separately owned canonical loader.
//! These DTOs are local stubs, not a second ingestion/rights implementation.
//! Replace them with the canonical loader types when available. No fallback to
//! permissive legacy string loading is allowed for explicitly canonical inputs.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Legacy loaders retain their existing Vec<String> signatures.
#[derive(Debug, Clone)]
pub enum WorkloadInput {
    Legacy(Vec<String>),
    Canonical(Vec<CanonicalRecord>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalRecord {
    pub record_id: String,
    pub source_row_id: String,
    pub session_id: Option<String>,
    pub turn_index: Option<u32>,
    pub group_id: Option<String>,
    pub trace_id: Option<String>,
    pub arrival_offset_ms: Option<u64>,
    pub body: CanonicalBody,
    pub expected_output_tokens: Option<u32>,
    pub source: DatasetEnvelopeRef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalBody {
    Prompt(String),
    Chat(Vec<Message>),
}

// Message mirrors the existing canonical message type until the loader lands.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub role: Role,
    pub content: String,
    pub tool_call_ids: Vec<String>,
    pub tool_result_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetEnvelopeRef {
    pub source_id: String,
    pub resolved_revision: Option<String>,
    pub config: Option<String>,
    pub split: Option<String>,
    pub files: Vec<FileRef>,
    pub adapter_id: Option<String>,
    pub adapter_version: Option<String>,
    pub license_spdx: Option<String>,
    pub validation_manifest: Option<ValidationManifestRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileRef {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationManifestRef {
    pub schema_version: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy)]
pub struct IngestionLimits {
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    pub max_rows: usize,
}

/// Intentionally unavailable until the canonical loader is integrated. Never
/// parse a canonical input as a legacy prompt file merely to keep a run going.
pub fn load_canonical_jsonl(
    _path: &Path,
    _limits: IngestionLimits,
) -> Result<Vec<CanonicalRecord>> {
    bail!("canonical input loader is not integrated; refusing traffic and legacy fallback")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_turn_type_rejects_overflow_instead_of_truncating() {
        let record = CanonicalRecord {
            record_id: "record-1".into(),
            source_row_id: "row-1".into(),
            session_id: None,
            turn_index: Some(u32::MAX),
            group_id: None,
            trace_id: None,
            arrival_offset_ms: None,
            body: CanonicalBody::Prompt("prompt".into()),
            expected_output_tokens: Some(8),
            source: DatasetEnvelopeRef {
                source_id: "fixture".into(),
                resolved_revision: None,
                config: None,
                split: None,
                files: vec![],
                adapter_id: None,
                adapter_version: None,
                license_spdx: None,
                validation_manifest: None,
            },
        };
        let mut value = serde_json::to_value(record).unwrap();
        let decoded: CanonicalRecord = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(decoded.turn_index, Some(u32::MAX));
        assert_eq!(decoded.source_row_id, "row-1");
        assert!(matches!(decoded.body, CanonicalBody::Prompt(text) if text == "prompt"));
        value["turn_index"] = serde_json::json!(u64::from(u32::MAX) + 1);
        assert!(serde_json::from_value::<CanonicalRecord>(value).is_err());
    }

    #[test]
    fn canonical_stub_cannot_silently_fall_back_to_legacy() {
        let error = load_canonical_jsonl(
            Path::new("not-read.jsonl"),
            IngestionLimits {
                max_file_bytes: 1024,
                max_total_bytes: 1024,
                max_rows: 1,
            },
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("refusing traffic and legacy fallback"));
        let input = WorkloadInput::Legacy(vec!["unchanged".into()]);
        assert!(matches!(input, WorkloadInput::Legacy(prompts) if prompts == ["unchanged"]));
    }
}
