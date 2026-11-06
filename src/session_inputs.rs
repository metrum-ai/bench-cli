// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Typed source transcripts, not unconditional runtime request points.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::canonical_input::{validate_messages, Message, Role, ToolCall};
use crate::public_dataset::{sha256, DatasetOrigin, HistoryMode, RightsPolicy};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SessionShape {
    Chat {
        messages_field: String,
        role_field: String,
        content_field: String,
        roles: BTreeMap<String, Role>,
        tool_calls_field: Option<String>,
        tool_call_id_field: Option<String>,
    },
    Turns {
        turns_field: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionMapping {
    pub adapter_id: String,
    pub adapter_version: String,
    pub expected_config: String,
    pub expected_split: String,
    pub row_id_field: String,
    pub history_mode: HistoryMode,
    pub expected_output_tokens: Option<u32>,
    pub shape: SessionShape,
}

#[derive(Debug, Clone, Copy)]
pub struct SessionLimits {
    pub max_sessions: usize,
    pub max_messages: usize,
    pub max_body_bytes: usize,
}

/// Full private source conversation including reference assistants. This is NOT
/// CanonicalRecord: only the session runner selects request boundaries/history.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionTranscript {
    pub source_row_id: String,
    pub source_file_sha256: String,
    pub session_id: String,
    pub history_mode: HistoryMode,
    pub messages: Vec<Message>,
    pub expected_output_tokens: Option<u32>,
    pub normalization_sha256: String,
}

fn identifier(value: &Value) -> Result<String> {
    match value {
        Value::String(id) if !id.trim().is_empty() => Ok(id.clone()),
        Value::Number(id) if id.is_u64() => Ok(id.to_string()),
        _ => anyhow::bail!("nonempty string or unsigned source identity required"),
    }
}

/// Normalize already checksum-verified source rows. The caller must use the
/// shared pinned file reader first; parsed Values alone cannot verify file bytes.
/// No request point, arrival timing, placeholder or tool execution is inferred.
pub fn normalize(
    rows: &[Value],
    origin: &DatasetOrigin,
    source_file_sha256: &str,
    rights: &RightsPolicy,
    mapping: &SessionMapping,
    limits: SessionLimits,
) -> Result<Vec<SessionTranscript>> {
    origin.validate()?;
    rights.authorize()?;
    ensure!(
        origin.config == mapping.expected_config && origin.split == mapping.expected_split,
        "session config/split mismatch"
    );
    ensure!(
        origin
            .files
            .iter()
            .any(|f| f.sha256.eq_ignore_ascii_case(source_file_sha256)),
        "session source file not in pinned origin"
    );
    ensure!(
        !mapping.adapter_id.is_empty() && !mapping.adapter_version.is_empty(),
        "session adapter identity required"
    );
    ensure!(
        mapping.expected_output_tokens.is_none_or(|n| n > 0),
        "output target must be positive"
    );
    ensure!(
        !rows.is_empty() && rows.len() <= limits.max_sessions,
        "session row budget or empty input"
    );
    let mut ids = BTreeSet::new();
    let mut total_messages = 0usize;
    let mut total_bytes = 0usize;
    let mut output = Vec::new();
    for row in rows {
        let source_row_id = identifier(
            row.get(&mapping.row_id_field)
                .context("missing session source identity")?,
        )?;
        let session_id = format!(
            "session-{}",
            sha256(&serde_json::to_vec(&(
                &origin.source_id,
                &origin.resolved_revision,
                &source_row_id
            ))?)
        );
        ensure!(ids.insert(session_id.clone()), "duplicate source session");
        let messages = match &mapping.shape {
            SessionShape::Chat {
                messages_field,
                role_field,
                content_field,
                roles,
                tool_calls_field,
                tool_call_id_field,
            } => {
                let source = row
                    .get(messages_field)
                    .and_then(Value::as_array)
                    .context("session messages must be an array")?;
                ensure!(
                    source.len() <= limits.max_messages.saturating_sub(total_messages),
                    "session message budget exceeded"
                );
                source
                    .iter()
                    .map(|raw| {
                        let role = raw
                            .get(role_field)
                            .and_then(Value::as_str)
                            .and_then(|r| roles.get(r))
                            .copied()
                            .context("unknown or missing mapped session role")?;
                        let content = match raw.get(content_field) {
                            None | Some(Value::Null) => None,
                            Some(Value::String(s)) => Some(s.clone()),
                            _ => anyhow::bail!("session content must be text or null"),
                        };
                        ensure!(
                            !raw.get("tool_calls").is_some_and(
                                |v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty())
                            ) || tool_calls_field.as_deref() == Some("tool_calls"),
                            "unmapped typed tool calls"
                        );
                        ensure!(
                            !raw.get("tool_call_id").is_some_and(|v| !v.is_null())
                                || tool_call_id_field.as_deref() == Some("tool_call_id"),
                            "unmapped tool result identity"
                        );
                        let calls = tool_calls_field.as_ref().and_then(|key| raw.get(key));
                        let tool_calls: Vec<ToolCall> = match calls {
                            None | Some(Value::Null) => vec![],
                            Some(value) => serde_json::from_value(value.clone())
                                .map_err(|_| anyhow::anyhow!("invalid typed source tool calls"))?,
                        };
                        let tool_call_id =
                            match tool_call_id_field.as_ref().and_then(|key| raw.get(key)) {
                                None | Some(Value::Null) => None,
                                Some(Value::String(id)) => Some(id.clone()),
                                _ => anyhow::bail!("invalid source tool result identity"),
                            };
                        Ok(Message {
                            role,
                            content,
                            tool_calls,
                            tool_call_id,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?
            }
            SessionShape::Turns { turns_field } => {
                let turns = row
                    .get(turns_field)
                    .and_then(Value::as_array)
                    .context("session turns must be an array")?;
                ensure!(
                    turns.len() <= limits.max_messages.saturating_sub(total_messages),
                    "session message budget exceeded"
                );
                ensure!(turns.len() <= 1 || mapping.history_mode == HistoryMode::Generated, "multi-turn prompt-only sources require generated history; no assistant placeholders are fabricated");
                turns
                    .iter()
                    .map(|turn| {
                        Ok(Message {
                            role: Role::User,
                            content: Some(
                                turn.as_str().context("session turn must be text")?.into(),
                            ),
                            tool_calls: vec![],
                            tool_call_id: None,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?
            }
        };
        ensure!(
            messages.iter().any(|m| m.role == Role::User),
            "session source must contain user history"
        );
        // A source transcript may end in a reference assistant, unlike a runtime
        // request. A validation-only user suffix checks the entire causal stream
        // without selecting a real request point or exporting the suffix.
        let mut checked = messages.clone();
        checked.push(Message {
            role: Role::User,
            content: Some("validation-only boundary".into()),
            tool_calls: vec![],
            tool_call_id: None,
        });
        validate_messages(&checked)?;
        total_messages = total_messages
            .checked_add(messages.len())
            .context("session message count overflow")?;
        total_bytes = total_bytes
            .checked_add(serde_json::to_vec(&messages)?.len())
            .context("session byte count overflow")?;
        ensure!(
            total_messages <= limits.max_messages && total_bytes <= limits.max_body_bytes,
            "aggregate session budget exceeded"
        );
        let normalization_sha256 = sha256(&serde_json::to_vec(&(
            mapping,
            source_file_sha256,
            &source_row_id,
            &messages,
        ))?);
        output.push(SessionTranscript {
            source_row_id,
            source_file_sha256: source_file_sha256.into(),
            session_id,
            history_mode: mapping.history_mode.clone(),
            messages,
            expected_output_tokens: mapping.expected_output_tokens,
            normalization_sha256,
        });
    }
    Ok(output)
}

/// Safe default evidence: no source identifiers, reference replies, tool arguments
/// or prompt text. Full-source normalization is not measured live-history reuse.
pub fn metadata(sessions: &[SessionTranscript]) -> Result<Value> {
    let mut roles: BTreeMap<String, usize> = BTreeMap::new();
    for session in sessions {
        for message in &session.messages {
            *roles
                .entry(
                    serde_json::to_value(message.role)?
                        .as_str()
                        .context("invalid role")?
                        .into(),
                )
                .or_default() += 1;
        }
    }
    Ok(
        json!({"schema_version":"session-normalization.v1", "sessions":sessions.len(), "roles":roles, "normalization_order_sha256":sha256(&serde_json::to_vec(&sessions.iter().map(|s| &s.normalization_sha256).collect::<Vec<_>>())?), "corpus_rows":{"status":"unknown"}, "arrivals":{"status":"unknown"}, "live_history_overlap":{"status":"unknown"}, "request_points":"owned by session runtime"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::public_dataset::SourceFile;
    fn setup() -> (DatasetOrigin, RightsPolicy, SessionMapping, SessionLimits) {
        let origin = DatasetOrigin {
            source_id: "example/synthetic-sessions".into(),
            resolved_revision: "a".repeat(40),
            config: "default".into(),
            split: "test".into(),
            files: vec![SourceFile {
                path: "fixture.jsonl".into(),
                sha256: "b".repeat(64),
                bytes: 1,
            }],
        };
        let rights = RightsPolicy {
            decision: crate::public_dataset::AccessDecision::Allowed,
            license: Some("Apache-2.0".into()),
            license_text_sha256: Some("c".repeat(64)),
            component_terms: BTreeMap::new(),
            gated: false,
            terms_review: None,
            redistribution_allowed: false,
            privacy_handling: "synthetic test only".into(),
            unresolved: vec![],
        };
        let mapping = SessionMapping {
            adapter_id: "synthetic-chat".into(),
            adapter_version: "1".into(),
            expected_config: "default".into(),
            expected_split: "test".into(),
            row_id_field: "id".into(),
            history_mode: HistoryMode::Recorded,
            expected_output_tokens: Some(8),
            shape: SessionShape::Chat {
                messages_field: "messages".into(),
                role_field: "role".into(),
                content_field: "content".into(),
                roles: BTreeMap::from([
                    ("user".into(), Role::User),
                    ("assistant".into(), Role::Assistant),
                    ("tool".into(), Role::Tool),
                ]),
                tool_calls_field: Some("tool_calls".into()),
                tool_call_id_field: Some("tool_call_id".into()),
            },
        };
        (
            origin,
            rights,
            mapping,
            SessionLimits {
                max_sessions: 8,
                max_messages: 32,
                max_body_bytes: 16384,
            },
        )
    }
    #[test]
    fn synthetic_ultrachat_shape_preserves_source_references_without_selecting_requests() {
        let (origin, rights, mapping, limits) = setup();
        let rows = vec![
            json!({"id":"private-source-id","messages":[{"role":"user","content":"FIRST"},{"role":"assistant","content":"REFERENCE"},{"role":"user","content":"NEXT"},{"role":"assistant","content":"FUTURE"}]}),
        ];
        let result = normalize(
            &rows,
            &origin,
            &origin.files[0].sha256,
            &rights,
            &mapping,
            limits,
        )
        .unwrap();
        assert_eq!(result[0].messages.len(), 4);
        assert_eq!(result[0].messages[3].content.as_deref(), Some("FUTURE"));
        assert_eq!(
            result,
            normalize(
                &rows,
                &origin,
                &origin.files[0].sha256,
                &rights,
                &mapping,
                limits
            )
            .unwrap()
        );
        let evidence = metadata(&result).unwrap().to_string();
        for raw in ["private-source-id", "FIRST", "REFERENCE", "FUTURE"] {
            assert!(!evidence.contains(raw));
        }
        assert!(evidence.contains("unknown"));
    }
    #[test]
    fn mtbench_style_turns_never_fabricate_assistants_or_arrivals() {
        let (origin, rights, mut mapping, limits) = setup();
        mapping.shape = SessionShape::Turns {
            turns_field: "turns".into(),
        };
        let rows = vec![json!({"id":1,"turns":["one","two"]})];
        assert!(normalize(
            &rows,
            &origin,
            &origin.files[0].sha256,
            &rights,
            &mapping,
            limits
        )
        .is_err());
        mapping.history_mode = HistoryMode::Generated;
        let result = normalize(
            &rows,
            &origin,
            &origin.files[0].sha256,
            &rights,
            &mapping,
            limits,
        )
        .unwrap();
        assert_eq!(result[0].messages.len(), 2);
        assert!(result[0].messages.iter().all(|m| m.role == Role::User));
        let mut wrong = mapping;
        wrong.shape = SessionShape::Turns {
            turns_field: "prompts".into(),
        };
        assert!(normalize(
            &rows,
            &origin,
            &origin.files[0].sha256,
            &rights,
            &wrong,
            limits
        )
        .is_err());
    }
    #[test]
    fn typed_tool_calls_are_preserved_and_never_executed() {
        let (origin, rights, mapping, limits) = setup();
        let row = json!({"id":"s","messages":[{"role":"user","content":"lookup"},{"role":"assistant","content":null,"tool_calls":[{"id":"c","type":"function","function":{"name":"never_execute","arguments":"{\"command\":\"SENTINEL\"}"}}]},{"role":"tool","content":"recorded result","tool_call_id":"c"},{"role":"assistant","content":"reference"}]});
        let result = normalize(
            std::slice::from_ref(&row),
            &origin,
            &origin.files[0].sha256,
            &rights,
            &mapping,
            limits,
        )
        .unwrap();
        assert_eq!(
            result[0].messages[1].tool_calls[0].function.name,
            "never_execute"
        );
        assert!(!metadata(&result).unwrap().to_string().contains("SENTINEL"));
        let mut bad = row;
        bad["messages"][2]["tool_call_id"] = json!("unmatched");
        assert!(normalize(
            &[bad],
            &origin,
            &origin.files[0].sha256,
            &rights,
            &mapping,
            limits
        )
        .is_err());
    }
    #[test]
    fn malformed_duplicates_limits_and_policy_fail_closed() {
        let (origin, rights, mapping, limits) = setup();
        let good = json!({"id":"s","messages":[{"role":"user","content":"hi"}]});
        assert!(normalize(
            &[good.clone(), good.clone()],
            &origin,
            &origin.files[0].sha256,
            &rights,
            &mapping,
            limits
        )
        .is_err());
        assert!(normalize(
            std::slice::from_ref(&good),
            &origin,
            &origin.files[0].sha256,
            &rights,
            &mapping,
            SessionLimits {
                max_messages: 0,
                ..limits
            }
        )
        .is_err());
        for messages in [
            json!([]),
            json!([{"role":"assistant","content":"not a user request"}]),
            json!([{"role":"unknown","content":"PRIVATE"}]),
            json!([{"role":"user","content":""}]),
        ] {
            let mut bad = good.clone();
            bad["messages"] = messages;
            assert!(normalize(
                &[bad],
                &origin,
                &origin.files[0].sha256,
                &rights,
                &mapping,
                limits
            )
            .is_err());
        }
        let mut denied = rights;
        denied.license = None;
        assert!(normalize(
            &[good],
            &origin,
            &origin.files[0].sha256,
            &denied,
            &mapping,
            limits
        )
        .is_err());
    }
}
