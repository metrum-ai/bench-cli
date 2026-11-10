// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Inert source-agent tool schema/argument mappings, not executors.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::canonical_input::{ToolCall, ToolCallType};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolArgumentsEncoding {
    #[default]
    SerializedJson,
    JsonObject,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolFunctionDefinition {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parameters: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub kind: ToolCallType,
    pub function: ToolFunctionDefinition,
}

pub(super) fn definitions(value: Option<&Value>) -> Result<Vec<ToolDefinition>> {
    let tools: Vec<ToolDefinition> = match value {
        None | Some(Value::Null) => Vec::new(),
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|_| anyhow::anyhow!("invalid source tool definitions"))?,
    };
    let mut names = std::collections::BTreeSet::new();
    for tool in &tools {
        ensure!(
            !tool.function.name.trim().is_empty() && names.insert(&tool.function.name),
            "missing or duplicate tool definition name"
        );
        ensure!(
            tool.function.parameters.is_object(),
            "tool parameters must be a JSON schema object"
        );
    }
    Ok(tools)
}

pub(super) fn calls(
    value: Option<&Value>,
    encoding: ToolArgumentsEncoding,
) -> Result<Vec<ToolCall>> {
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return Ok(Vec::new());
    };
    let mut calls = value
        .as_array()
        .context("source tool calls must be an array")?
        .clone();
    if encoding == ToolArgumentsEncoding::JsonObject {
        for call in &mut calls {
            let args = call
                .get_mut("function")
                .and_then(|v| v.get_mut("arguments"))
                .context("source tool arguments required")?;
            ensure!(args.is_object(), "declared object tool arguments required");
            *args = Value::String(serde_json::to_string(args)?);
        }
    }
    serde_json::from_value(Value::Array(calls))
        .map_err(|_| anyhow::anyhow!("invalid typed source tool calls"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn declared_object_arguments_become_inert_canonical_json_strings() {
        let input = json!([{"id":"c","type":"function","function":{"name":"never_execute","arguments":{"command":"SENTINEL"}}}]);
        assert!(calls(Some(&input), ToolArgumentsEncoding::SerializedJson).is_err());
        let normalized = calls(Some(&input), ToolArgumentsEncoding::JsonObject).unwrap();
        assert_eq!(normalized[0].function.name, "never_execute");
        assert_eq!(
            serde_json::from_str::<Value>(&normalized[0].function.arguments).unwrap(),
            json!({"command":"SENTINEL"})
        );
        assert!(calls(
            Some(&json!([{"function":{"arguments":[]}}])),
            ToolArgumentsEncoding::JsonObject
        )
        .is_err());
    }
    #[test]
    fn tool_definitions_are_typed_and_never_silently_discarded() {
        let input = json!([{"type":"function","function":{"name":"lookup","parameters":{"type":"object","properties":{"x":{"type":"string"}}},"strict":true}}]);
        assert_eq!(
            serde_json::to_value(definitions(Some(&input)).unwrap()).unwrap(),
            input
        );
        assert!(definitions(Some(&json!([{"type":"shell","function":{}}]))).is_err());
        assert!(definitions(Some(
            &json!([{"type":"function","function":{"name":"lookup","parameters":[]}}])
        ))
        .is_err());
        let duplicated = json!([input[0], input[0]]);
        assert!(definitions(Some(&duplicated)).is_err());
    }
}
