// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Temporary DTO/loader stubs matching the canonical runtime interface. Replace
//! this module with the canonical implementation when integrated. Consumption
//! wrappers live in workload_input; never duplicate fetching/rights validation.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

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
pub enum CanonicalBody {
    Prompt(String),
    Chat(Vec<Message>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub role: Role,
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallType {
    Function,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: ToolCallType,
    pub function: RecordedFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetEnvelopeRef {
    pub source_id: String,
    pub resolved_revision: Option<String>,
    pub config: Option<String>,
    pub split: Option<String>,
    pub files: Vec<SourceFile>,
    pub adapter_id: Option<String>,
    pub adapter_version: Option<String>,
    pub license_spdx: Option<String>,
    pub validation_manifest: Option<ValidationManifestRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceFile {
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
    }
}
