// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Bounded canonical runtime inputs. Legacy string loaders remain unchanged.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};

pub use crate::public_dataset::{IngestionLimits, SourceFile, ValidationManifestRef};

/// The exact study-report datasets[] envelope; null retains unknown provenance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

impl From<crate::public_dataset::DatasetEnvelope> for DatasetEnvelopeRef {
    fn from(source: crate::public_dataset::DatasetEnvelope) -> Self {
        Self {
            source_id: source.source_id,
            resolved_revision: Some(source.resolved_revision),
            config: Some(source.config),
            split: Some(source.split),
            files: source.files,
            adapter_id: Some(source.adapter_id),
            adapter_version: Some(source.adapter_version),
            license_spdx: source.license_spdx,
            validation_manifest: Some(source.validation_manifest),
        }
    }
}

/// Text-only input roles; a tool result is not a tool executor.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// Supported tool-call type, deliberately not an arbitrary action enum.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallType {
    Function,
}

/// Recorded function metadata. Arguments remain inert serialized JSON.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecordedFunction {
    pub name: String,
    pub arguments: String,
}

/// Recorded tool-call identity and data; never dispatch this during ingestion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: ToolCallType,
    pub function: RecordedFunction,
}

/// Canonical OpenAI-shaped text message, preserving typed tool calls/results.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub role: Role,
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// Runtime input contract. Single prompts become one User message. Source-row
/// lineage is held separately in ingestion validation, not dropped into text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalRecord {
    pub record_id: String,
    pub session_id: Option<String>,
    pub turn_index: Option<u32>,
    pub group_id: Option<String>,
    pub trace_id: Option<String>,
    pub arrival_offset_ms: Option<u64>,
    pub messages: Vec<Message>,
    pub expected_output_tokens: Option<u32>,
    pub source: DatasetEnvelopeRef,
}

fn hex(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|b| b.is_ascii_hexdigit())
}

impl CanonicalRecord {
    /// Structural/causal validation only. A reference digest is not legal clearance
    /// or verification of the referenced artifact's content; no fetching occurs.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.record_id.trim().is_empty(),
            "canonical record ID required"
        );
        for id in [&self.session_id, &self.group_id, &self.trace_id] {
            ensure!(
                id.as_ref().is_none_or(|s| !s.trim().is_empty()),
                "empty canonical identity"
            );
        }
        ensure!(
            self.turn_index.is_none() || self.session_id.is_some(),
            "turn index requires session ID"
        );
        ensure!(
            self.expected_output_tokens.is_none_or(|n| n > 0),
            "output token target must be positive"
        );
        ensure!(
            !self.source.source_id.trim().is_empty(),
            "source ID required"
        );
        for value in [
            &self.source.resolved_revision,
            &self.source.config,
            &self.source.split,
            &self.source.adapter_id,
            &self.source.adapter_version,
            &self.source.license_spdx,
        ] {
            ensure!(
                value.as_ref().is_none_or(|s| !s.trim().is_empty()),
                "empty source metadata must be null"
            );
        }
        let mut paths = BTreeSet::new();
        for file in &self.source.files {
            ensure!(
                !file.path.is_empty() && paths.insert(&file.path),
                "empty or duplicate source file path"
            );
            ensure!(hex(&file.sha256, 64), "invalid source digest");
        }
        if let Some(reference) = &self.source.validation_manifest {
            ensure!(
                reference.schema_version == crate::public_dataset::SCHEMA_VERSION
                    && hex(&reference.sha256, 64),
                "invalid dataset validation reference"
            );
        }
        validate_messages(&self.messages)
    }
}

/// Check inert message causality. Requests may end at a user or resolved tool
/// result, never at a future/reference assistant answer or system-only prefix.
pub fn validate_messages(messages: &[Message]) -> Result<()> {
    ensure!(!messages.is_empty(), "canonical messages required");
    let mut pending: BTreeSet<String> = BTreeSet::new();
    let mut used = BTreeSet::new();
    let mut user_seen = false;
    for message in messages {
        ensure!(
            message
                .content
                .as_ref()
                .is_some_and(|s| !s.trim().is_empty())
                || !message.tool_calls.is_empty(),
            "empty canonical message"
        );
        ensure!(
            message.tool_calls.is_empty() || message.role == Role::Assistant,
            "only assistant may call tools"
        );
        ensure!(
            message.tool_call_id.is_none() || message.role == Role::Tool,
            "only tool may supply call ID"
        );
        if message.role == Role::Tool {
            let id = message
                .tool_call_id
                .as_ref()
                .context("tool result call ID required")?;
            ensure!(pending.remove(id), "unmatched or duplicate tool result");
        } else {
            ensure!(
                pending.is_empty(),
                "unresolved tools before next conversation message"
            );
        }
        match message.role {
            Role::System => ensure!(!user_seen, "system messages must precede conversation"),
            Role::User => user_seen = true,
            Role::Assistant => ensure!(user_seen, "assistant-only prefix is not a request"),
            Role::Tool => ensure!(user_seen, "tool-only prefix is not a request"),
        }
        for call in &message.tool_calls {
            ensure!(
                !call.id.is_empty() && used.insert(&call.id) && !call.function.name.is_empty(),
                "empty or duplicate tool call identity"
            );
            ensure!(
                serde_json::from_str::<serde_json::Value>(&call.function.arguments)
                    .is_ok_and(|v| v.is_object()),
                "tool arguments must be a JSON object"
            );
            pending.insert(call.id.clone());
        }
    }
    ensure!(
        pending.is_empty(),
        "unresolved tool calls at request boundary"
    );
    ensure!(
        messages
            .last()
            .is_some_and(|m| matches!(m.role, Role::User | Role::Tool)),
        "request cannot end in assistant reference answer or system-only prefix"
    );
    Ok(())
}

/// Load only local bounded JSONL, preserving exact source order. No remote URLs,
/// agreement acceptance, tool execution or fallback to legacy string parsing.
pub fn load_canonical_jsonl(
    path: impl AsRef<Path>,
    limits: IngestionLimits,
) -> Result<Vec<CanonicalRecord>> {
    let path = path.as_ref();
    let size = std::fs::metadata(path)?.len();
    ensure!(
        size <= limits.max_file_bytes.min(limits.max_total_bytes),
        "canonical file byte budget exceeded"
    );
    let rows = crate::public_dataset::read_jsonl(std::fs::File::open(path)?, limits)?;
    let mut records = Vec::with_capacity(rows.len());
    let mut ids = BTreeSet::new();
    let mut session_turns = BTreeSet::new();
    for (i, row) in rows.into_iter().enumerate() {
        // Do not echo raw messages, unknown role strings or tool arguments in errors.
        let record: CanonicalRecord = serde_json::from_value(row)
            .map_err(|_| anyhow::anyhow!("invalid canonical schema at source row {}", i + 1))?;
        record
            .validate()
            .with_context(|| format!("invalid canonical source row {}", i + 1))?;
        ensure!(
            ids.insert(record.record_id.clone()),
            "duplicate canonical record ID"
        );
        if let (Some(session), Some(turn)) = (&record.session_id, record.turn_index) {
            ensure!(
                session_turns.insert((session.clone(), turn)),
                "duplicate session turn"
            );
        }
        records.push(record);
    }
    ensure!(!records.is_empty(), "empty canonical workload");
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    fn fixture(id: &str) -> Value {
        json!({"record_id":id, "session_id":"session", "turn_index":0, "group_id":"group", "trace_id":null, "arrival_offset_ms":42, "messages":[{"role":"user","content":"héllo"}], "expected_output_tokens":8, "source":{"source_id":"synthetic:fixture", "resolved_revision":null,"config":null,"split":null,"files":[],"adapter_id":"fixture","adapter_version":"1","license_spdx":"Apache-2.0","validation_manifest":null}})
    }
    fn load(rows: &[Value], limits: IngestionLimits) -> Result<Vec<CanonicalRecord>> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.jsonl");
        let data = rows
            .iter()
            .map(|r| serde_json::to_string(r).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, data).unwrap();
        load_canonical_jsonl(path, limits)
    }
    fn limits() -> IngestionLimits {
        IngestionLimits {
            max_file_bytes: 16384,
            max_total_bytes: 16384,
            max_rows: 8,
        }
    }
    #[test]
    fn exact_names_types_order_and_limits() {
        let a = fixture("a");
        let mut b = fixture("b");
        b["turn_index"] = json!(1);
        let rows = load(&[a.clone(), b.clone()], limits()).unwrap();
        assert_eq!(rows[0].record_id, "a");
        assert_eq!(rows[1].record_id, "b");
        assert_eq!(rows[0].arrival_offset_ms, Some(42));
        assert_eq!(rows[0].messages[0].content.as_deref(), Some("héllo"));
        let _: Option<u32> = rows[0].turn_index;
        let _: Option<u32> = rows[0].expected_output_tokens;
        assert!(load(
            &[a.clone(), b],
            IngestionLimits {
                max_rows: 1,
                ..limits()
            }
        )
        .is_err());
        assert!(load(
            std::slice::from_ref(&a),
            IngestionLimits {
                max_total_bytes: 1,
                ..limits()
            }
        )
        .is_err());
        assert!(load(&[a.clone(), a], limits()).is_err());
        assert!(load(&[], limits()).is_err());
    }
    #[test]
    fn schema_errors_and_future_answers_are_rejected_without_payload_echo() {
        let mut bad = fixture("a");
        bad["turn_index"] = json!(4_294_967_296u64);
        assert!(load(&[bad], limits()).is_err());
        let mut bad = fixture("a");
        bad["messages"][0]["role"] = json!("SENSITIVE_SENTINEL");
        let error = load(&[bad], limits()).unwrap_err();
        assert!(!format!("{error:#}").contains("SENSITIVE_SENTINEL"));
        let mut bad = fixture("a");
        bad["messages"] = json!([{"role":"assistant","content":"future answer"}]);
        assert!(load(&[bad], limits()).is_err());
        let mut bad = fixture("a");
        bad["messages"] = json!([{"role":"user","content":"question"},{"role":"assistant","content":"future answer"}]);
        assert!(load(&[bad], limits()).is_err());
        let mut bad = fixture("a");
        bad["source"]["validation_manifest"] =
            json!({"schema_version":"kv-dataset-manifest.v1","sha256":"invalid"});
        assert!(load(&[bad], limits()).is_err());
    }
    #[test]
    fn typed_tool_data_is_preserved_but_never_executed() {
        let mut row = fixture("tool-request");
        row["messages"] = json!([
            {"role":"user","content":"look up"},
            {"role":"assistant","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"never_execute","arguments":"{\"command\":\"SENTINEL_NOT_EXECUTED\"}"}}]},
            {"role":"tool","content":"recorded result","tool_call_id":"call-1"}
        ]);
        let loaded = load(std::slice::from_ref(&row), limits()).unwrap();
        assert_eq!(
            loaded[0].messages[1].tool_calls[0].function.name,
            "never_execute"
        );
        row["messages"][2]["tool_call_id"] = json!("unmatched");
        assert!(load(&[row], limits()).is_err());
    }
}
