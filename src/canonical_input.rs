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

/// Prompt-only and chat inputs remain distinct; never discard source-row lineage.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CanonicalBody {
    Prompt(String),
    Chat(Vec<Message>),
}

/// Final runtime contract shared by ingestion and runner consumers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

/// Checked conversion requires the source envelope explicitly: ingestion records
/// do not own enough provenance to invent one. See docs/PUBLIC_DATASETS.md.
impl TryFrom<(crate::public_dataset::IngestionRecord, DatasetEnvelopeRef)> for CanonicalRecord {
    type Error = anyhow::Error;

    fn try_from(
        (record, source): (crate::public_dataset::IngestionRecord, DatasetEnvelopeRef),
    ) -> Result<Self> {
        use crate::public_dataset::{HistoryMode, IngestionBody, Role as SourceRole};
        ensure!(
            source
                .files
                .iter()
                .any(|f| f.sha256.eq_ignore_ascii_case(&record.source_file_sha256)),
            "ingestion source file is missing from runtime envelope"
        );
        let turn_index = record
            .turn_index
            .map(u32::try_from)
            .transpose()
            .context("runtime turn_index exceeds u32")?;
        let expected_output_tokens = record
            .output_budget
            .map(u32::try_from)
            .transpose()
            .context("runtime expected_output_tokens exceeds u32")?;
        let body = match record.body {
            IngestionBody::Prompt { text } => CanonicalBody::Prompt(text),
            IngestionBody::DocumentQuestion { document_id, document, question } => {
                ensure!(!document_id.trim().is_empty() && !document.trim().is_empty() && !question.trim().is_empty(), "incomplete document/question input");
                CanonicalBody::Chat(vec![Message { role: Role::User, content: Some(format!("{document}\n\nQuestion:\n{question}")), tool_calls: vec![], tool_call_id: None }])
            }
            IngestionBody::Session { session_id, history_mode, messages } => {
                ensure!(record.session_id.as_ref() == Some(&session_id), "missing or conflicting session identity");
                ensure!(history_mode == HistoryMode::Recorded, "generated history must be materialized by the session runner before conversion");
                let messages = messages.into_iter().map(|message| {
                    ensure!(message.tool_call_ids.is_empty(), "tool call IDs alone cannot fabricate typed function metadata");
                    Ok(Message { role: match message.role { SourceRole::System => Role::System, SourceRole::User => Role::User, SourceRole::Assistant => Role::Assistant, SourceRole::Tool => Role::Tool }, content: Some(message.content), tool_calls: vec![], tool_call_id: message.tool_result_id })
                }).collect::<Result<Vec<_>>>()?;
                CanonicalBody::Chat(messages)
            }
            IngestionBody::StructuralTrace { .. } => anyhow::bail!("structural traces require an explicit validated hash-to-text conversion; original text is unavailable"),
        };
        let result = Self {
            record_id: record.record_id,
            source_row_id: record.source_row_id,
            session_id: record.session_id,
            turn_index,
            group_id: record.group_id,
            trace_id: record.trace_id,
            arrival_offset_ms: None,
            body,
            expected_output_tokens,
            source,
        };
        result.validate()?;
        Ok(result)
    }
}

impl crate::public_dataset::IngestionRecord {
    /// Convert a selected ingestion target with an explicit source envelope.
    /// No history selection, truncation, source inference or tool synthesis occurs.
    pub fn into_canonical(self, source: DatasetEnvelopeRef) -> Result<CanonicalRecord> {
        (self, source).try_into()
    }
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
        ensure!(
            !self.source_row_id.trim().is_empty(),
            "canonical source row ID required"
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
        match &self.body {
            CanonicalBody::Prompt(prompt) => {
                ensure!(!prompt.trim().is_empty(), "canonical prompt required");
                Ok(())
            }
            CanonicalBody::Chat(messages) => validate_messages(messages),
        }
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
        json!({"record_id":id, "source_row_id":"source-row-1", "session_id":"session", "turn_index":0, "group_id":"group", "trace_id":null, "arrival_offset_ms":42, "body":{"Chat":[{"role":"user","content":"héllo"}]},  "expected_output_tokens":8, "source":{"source_id":"synthetic:fixture", "resolved_revision":null,"config":null,"split":null,"files":[],"adapter_id":"fixture","adapter_version":"1","license_spdx":"Apache-2.0","validation_manifest":null}})
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
        assert_eq!(rows[0].source_row_id, "source-row-1");
        let CanonicalBody::Chat(messages) = &rows[0].body else {
            panic!("expected Chat")
        };
        assert_eq!(messages[0].content.as_deref(), Some("héllo"));
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
        bad["body"]["Chat"][0]["role"] = json!("SENSITIVE_SENTINEL");
        let error = load(&[bad], limits()).unwrap_err();
        assert!(!format!("{error:#}").contains("SENSITIVE_SENTINEL"));
        let mut bad = fixture("a");
        bad["body"] = json!({"Chat":[{"role":"assistant","content":"future answer"}]});
        assert!(load(&[bad], limits()).is_err());
        let mut bad = fixture("a");
        bad["body"] = json!({"Chat":[{"role":"user","content":"question"},{"role":"assistant","content":"future answer"}]});
        assert!(load(&[bad], limits()).is_err());
        let mut bad = fixture("a");
        bad["source"]["validation_manifest"] =
            json!({"schema_version":"kv-dataset-manifest.v1","sha256":"invalid"});
        assert!(load(&[bad], limits()).is_err());
    }
    #[test]
    fn prompt_chat_tags_required_lineage_and_integer_bounds() {
        let mut row = fixture("prompt");
        row["body"] = json!({"Prompt":"héllo\n"});
        row["arrival_offset_ms"] = Value::Null;
        let record = load(std::slice::from_ref(&row), limits())
            .unwrap()
            .remove(0);
        assert_eq!(record.body, CanonicalBody::Prompt("héllo\n".into()));
        assert_eq!(serde_json::to_value(&record).unwrap(), row);
        let _: Option<u64> = record.arrival_offset_ms;
        let _: &DatasetEnvelopeRef = &record.source;
        for field in ["source_row_id", "source", "body"] {
            let mut bad = row.clone();
            bad.as_object_mut().unwrap().remove(field);
            assert!(load(&[bad], limits()).is_err());
        }
        for value in [json!(""), json!(" ")] {
            let mut bad = row.clone();
            bad["source_row_id"] = value;
            assert!(load(&[bad], limits()).is_err());
        }
        for body in [
            json!({"Prompt":" "}),
            json!({"Chat":[]}),
            json!({"prompt":"wrong tag"}),
        ] {
            let mut bad = row.clone();
            bad["body"] = body;
            assert!(load(&[bad], limits()).is_err());
        }
        for field in ["turn_index", "expected_output_tokens"] {
            for value in [json!(-1), json!(4_294_967_296u64), json!(1.5)] {
                let mut bad = row.clone();
                bad[field] = value;
                assert!(load(&[bad], limits()).is_err());
            }
            let mut boundary = row.clone();
            boundary[field] = json!(u32::MAX);
            assert!(load(&[boundary], limits()).is_ok());
        }
        let mut legacy = row;
        legacy["messages"] = json!([]);
        assert!(load(&[legacy], limits()).is_err());
    }
    fn ingestion_fixture() -> (crate::public_dataset::IngestionRecord, DatasetEnvelopeRef) {
        use crate::public_dataset::{IngestionBody, IngestionRecord};
        let mut source: DatasetEnvelopeRef =
            serde_json::from_value(fixture("r")["source"].clone()).unwrap();
        source.files.push(SourceFile {
            path: "fixture.jsonl".into(),
            sha256: "a".repeat(64),
            bytes: 1,
        });
        (
            IngestionRecord {
                record_id: "r".into(),
                source_row_id: "source-1".into(),
                session_id: None,
                trace_id: Some("trace".into()),
                source_file_sha256: "a".repeat(64),
                group_id: Some("group".into()),
                turn_index: None,
                output_budget: Some(8),
                body: IngestionBody::Prompt {
                    text: "héllo\n".into(),
                },
            },
            source,
        )
    }
    #[test]
    fn checked_conversion_preserves_lineage_prompt_and_document_chat() {
        use crate::public_dataset::IngestionBody;
        let (mut input, source) = ingestion_fixture();
        let converted = input.clone().into_canonical(source.clone()).unwrap();
        assert_eq!(converted.body, CanonicalBody::Prompt("héllo\n".into()));
        assert_eq!(converted.source_row_id, input.source_row_id);
        assert_eq!(converted.record_id, input.record_id);
        assert_eq!(converted.group_id, input.group_id);
        assert_eq!(converted.trace_id, input.trace_id);
        assert_eq!(converted.expected_output_tokens, Some(8));
        assert_eq!(converted.arrival_offset_ms, None);
        assert_eq!(converted.source, source);
        input.body = IngestionBody::DocumentQuestion {
            document_id: "d".into(),
            document: "context".into(),
            question: "question".into(),
        };
        let converted: CanonicalRecord = (input, source).try_into().unwrap();
        let CanonicalBody::Chat(messages) = converted.body else {
            panic!("expected Chat")
        };
        assert_eq!(
            messages[0].content.as_deref(),
            Some("context\n\nQuestion:\nquestion")
        );
        assert_eq!(messages[0].role, Role::User);
    }
    #[test]
    fn checked_conversion_rejects_overflow_missing_fields_and_unmaterialized_bodies() {
        use crate::public_dataset::{
            HistoryMode, IngestionBody, Message as SourceMessage, Role as SourceRole, TimeUnit,
        };
        let (input, source) = ingestion_fixture();
        let mut overflow = input.clone();
        overflow.session_id = Some("s".into());
        overflow.turn_index = Some(u64::from(u32::MAX) + 1);
        assert!(overflow.into_canonical(source.clone()).is_err());
        let mut overflow = input.clone();
        overflow.output_budget = Some(u64::from(u32::MAX) + 1);
        assert!(overflow.into_canonical(source.clone()).is_err());
        for missing in ["record", "row", "digest"] {
            let mut bad = input.clone();
            match missing {
                "record" => bad.record_id.clear(),
                "row" => bad.source_row_id.clear(),
                _ => bad.source_file_sha256.clear(),
            }
            assert!(bad.into_canonical(source.clone()).is_err());
        }
        let mut missing = source.clone();
        missing.files.clear();
        assert!(input.clone().into_canonical(missing).is_err());
        let mut missing = source.clone();
        missing.source_id.clear();
        assert!(input.clone().into_canonical(missing).is_err());
        let mut session = input.clone();
        session.session_id = Some("s".into());
        session.turn_index = Some(u64::from(u32::MAX));
        session.body = IngestionBody::Session {
            session_id: "s".into(),
            history_mode: HistoryMode::Recorded,
            messages: vec![SourceMessage {
                role: SourceRole::User,
                content: "question".into(),
                tool_call_ids: vec![],
                tool_result_id: None,
            }],
        };
        let converted = session.clone().into_canonical(source.clone()).unwrap();
        assert_eq!(converted.turn_index, Some(u32::MAX));
        let mut missing = session.clone();
        missing.session_id = None;
        assert!(missing.into_canonical(source.clone()).is_err());
        if let IngestionBody::Session { history_mode, .. } = &mut session.body {
            *history_mode = HistoryMode::Generated;
        }
        assert!(session.clone().into_canonical(source.clone()).is_err());
        if let IngestionBody::Session {
            history_mode,
            messages,
            ..
        } = &mut session.body
        {
            *history_mode = HistoryMode::Recorded;
            messages.push(SourceMessage {
                role: SourceRole::Assistant,
                content: String::new(),
                tool_call_ids: vec!["incomplete-call".into()],
                tool_result_id: None,
            });
        }
        assert!(session.into_canonical(source.clone()).is_err());
        let mut trace = input;
        trace.body = IngestionBody::StructuralTrace {
            timestamp: 1,
            time_unit: TimeUnit::Milliseconds,
            input_tokens: 1,
            output_tokens: 1,
            block_size: None,
            prefix_hashes: None,
            post_template: true,
        };
        assert!(trace.into_canonical(source).is_err());
    }
    #[test]
    fn rust_and_documented_contracts_are_identical() {
        let source = include_str!("canonical_input.rs");
        let docs = include_str!("../docs/CANONICAL_INPUTS.md");
        for start in ["pub struct CanonicalRecord {", "pub enum CanonicalBody {"] {
            let fields = source
                .split_once(start)
                .unwrap()
                .1
                .split_once("\n}")
                .unwrap()
                .0;
            assert!(
                docs.contains(&format!("{start}{fields}\n}}")),
                "documented runtime contract drift"
            );
        }
        let row = fixture("chat");
        let record = load(std::slice::from_ref(&row), limits())
            .unwrap()
            .remove(0);
        assert_eq!(serde_json::to_value(record).unwrap(), row);
    }
    #[test]
    fn typed_tool_data_is_preserved_but_never_executed() {
        let mut row = fixture("tool-request");
        row["body"] = json!({"Chat":[
            {"role":"user","content":"look up"},
            {"role":"assistant","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"never_execute","arguments":"{\"command\":\"SENTINEL_NOT_EXECUTED\"}"}}]},
            {"role":"tool","content":"recorded result","tool_call_id":"call-1"}
        ]});
        let loaded = load(std::slice::from_ref(&row), limits()).unwrap();
        let CanonicalBody::Chat(messages) = &loaded[0].body else {
            panic!("expected Chat")
        };
        assert_eq!(messages[1].tool_calls[0].function.name, "never_execute");
        row["body"]["Chat"][2]["tool_call_id"] = json!("unmatched");
        assert!(load(&[row], limits()).is_err());
    }
}
