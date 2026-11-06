// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Consumption-owned wrappers. The canonical loader/types live separately in
//! canonical_input; replacing its temporary stub must not duplicate ingestion.

use crate::canonical_input::{CanonicalRecord, DatasetEnvelopeRef};
use serde::{Deserialize, Serialize};

/// Legacy loaders keep their existing signatures and scheduling semantics.
#[derive(Debug, Clone)]
pub enum WorkloadInput {
    Legacy(Vec<String>),
    Canonical(Vec<CanonicalRecord>),
}

/// Output lineage contains no messages or prompt text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalLineage {
    pub schema_version: String,
    pub record_id: String,
    pub source_row_id: String,
    pub session_id: Option<String>,
    pub turn_index: Option<u32>,
    pub group_id: Option<String>,
    pub trace_id: Option<String>,
    pub arrival_offset_ms: Option<u64>,
    pub expected_output_tokens: Option<u32>,
    pub source: DatasetEnvelopeRef,
}

impl From<&CanonicalRecord> for CanonicalLineage {
    fn from(record: &CanonicalRecord) -> Self {
        Self {
            schema_version: "canonical-lineage.v1".into(),
            record_id: record.record_id.clone(),
            source_row_id: record.source_row_id.clone(),
            session_id: record.session_id.clone(),
            turn_index: record.turn_index,
            group_id: record.group_id.clone(),
            trace_id: record.trace_id.clone(),
            arrival_offset_ms: record.arrival_offset_ms,
            expected_output_tokens: record.expected_output_tokens,
            source: record.source.clone(),
        }
    }
}

/// Wire preparation is distinct from ingestion: no source fetching, rights
/// inference, prefix expansion, tool execution or generated-history synthesis.
/// Deliberately not Serialize: messages can contain sensitive prompt content.
pub struct PreparedCanonicalInput {
    pub messages: Vec<serde_json::Value>,
    pub lineage: CanonicalLineage,
}

pub fn prepare_canonical_record(record: CanonicalRecord) -> anyhow::Result<PreparedCanonicalInput> {
    use crate::canonical_input::CanonicalBody;
    anyhow::ensure!(
        !record.record_id.is_empty() && !record.source_row_id.is_empty(),
        "canonical identity is required"
    );
    let lineage = CanonicalLineage::from(&record);
    let messages = match record.body {
        CanonicalBody::Prompt(prompt) => vec![serde_json::json!({"role":"user","content":prompt})],
        CanonicalBody::Chat(messages) => messages
            .into_iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()?,
    };
    anyhow::ensure!(!messages.is_empty(), "canonical chat has no messages");
    for message in &messages {
        let object = message
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("canonical message must be an object"))?;
        anyhow::ensure!(
            object.keys().all(|key| matches!(
                key.as_str(),
                "role" | "content" | "tool_calls" | "tool_call_id"
            )),
            "canonical message contains unsupported wire fields; refusing lossy conversion"
        );
    }
    Ok(PreparedCanonicalInput { messages, lineage })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical_input::CanonicalBody;

    fn record() -> CanonicalRecord {
        CanonicalRecord {
            record_id: "record-1".into(),
            source_row_id: "row-1".into(),
            session_id: Some("session-1".into()),
            turn_index: Some(u32::MAX),
            group_id: Some("group-1".into()),
            trace_id: Some("trace-1".into()),
            arrival_offset_ms: Some(100),
            body: CanonicalBody::Prompt("not for lineage export".into()),
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
        }
    }

    #[test]
    fn lineage_preserves_all_identities_and_never_copies_prompt_content() {
        let source = record();
        let value = serde_json::to_value(CanonicalLineage::from(&source)).unwrap();
        for field in [
            "record_id",
            "source_row_id",
            "session_id",
            "turn_index",
            "group_id",
            "trace_id",
            "arrival_offset_ms",
            "expected_output_tokens",
            "source",
        ] {
            assert_eq!(value[field], serde_json::to_value(&source).unwrap()[field]);
        }
        assert_eq!(value["schema_version"], "canonical-lineage.v1");
        for forbidden in ["body", "messages", "prompt"] {
            assert!(value.get(forbidden).is_none());
        }
        assert!(!value.to_string().contains("not for lineage export"));
    }

    #[test]
    fn preparation_preserves_prompt_and_does_not_expand_recorded_targets() {
        let prepared = prepare_canonical_record(record()).unwrap();
        assert_eq!(
            prepared.messages,
            vec![serde_json::json!({"role":"user","content":"not for lineage export"})]
        );
        assert_eq!(prepared.lineage.record_id, "record-1");
        assert_eq!(prepared.lineage.turn_index, Some(u32::MAX));
        assert_eq!(prepared.lineage.arrival_offset_ms, Some(100));
    }

    #[test]
    fn turn_overflow_is_rejected_without_lossy_conversion() {
        let mut value = serde_json::to_value(record()).unwrap();
        assert_eq!(
            serde_json::from_value::<CanonicalRecord>(value.clone())
                .unwrap()
                .turn_index,
            Some(u32::MAX)
        );
        value["turn_index"] = serde_json::json!(u64::from(u32::MAX) + 1);
        assert!(serde_json::from_value::<CanonicalRecord>(value).is_err());
        assert!(
            matches!(WorkloadInput::Legacy(vec!["unchanged".into()]), WorkloadInput::Legacy(prompts) if prompts == ["unchanged"])
        );
    }
}
