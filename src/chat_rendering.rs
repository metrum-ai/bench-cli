// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Local pinned chat-template rendering of exact final HTTP JSON body bytes.

use std::io::{Read, Write};
use std::path::Path;

use anyhow::{ensure, Context, Result};
use minijinja::{value::Serde, Environment, Error, ErrorKind, UndefinedBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::prefix_validation::RenderingIdentity;
use crate::public_dataset::sha256;

#[path = "chat_template_context.rs"]
mod template_context;

/// Version of the local rendering algorithm (dependency versions are locked).
pub const RENDERER_VERSION: &str = "minijinja-chat.v2";

/// Explicit CPU/output budgets. Templates are operator-reviewed pinned artifacts,
/// not arbitrary dataset code; fuel is not a general memory sandbox.
#[derive(Debug, Clone, Copy)]
pub struct RenderLimits {
    pub max_artifact_bytes: u64,
    pub max_body_bytes: usize,
    pub max_rendered_bytes: usize,
    pub max_tokens: usize,
    pub template_fuel: u64,
}

/// Keep wire-body integrity separate from rendered-text integrity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderedChat {
    pub body_sha256: String,
    pub rendered_text_sha256: String,
    pub token_ids: Vec<u32>,
    pub identity: RenderingIdentity,
}

struct BoundedOutput {
    bytes: Vec<u8>,
    max: usize,
}
impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.max.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::other("rendered byte budget exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn artifact(path: &Path, max: u64) -> Result<Vec<u8>> {
    ensure!(
        std::fs::metadata(path)?.len() <= max,
        "render artifact byte budget exceeded"
    );
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(max.saturating_add(1))
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= max,
        "render artifact grew beyond byte budget"
    );
    Ok(bytes)
}

/// Build the inert HF-style context. OpenAI wire arguments are serialized JSON;
/// HF chat templates access function.arguments as an object. Decode only this
/// representation, never dispatch a function or rewrite the issued wire bytes.
fn template_messages(body: &Value) -> Result<Value> {
    let mut messages = body["messages"]
        .as_array()
        .context("messages must be an array")?
        .clone();
    if let Some(tools) = body.get("tools").filter(|v| !v.is_null()) {
        for tool in tools.as_array().context("tools must be an array")? {
            ensure!(
                tool["type"] == "function" && tool["function"].is_object(),
                "unsupported tool definition"
            );
            ensure!(
                tool["function"]["name"]
                    .as_str()
                    .is_some_and(|v| !v.is_empty()),
                "tool function name required"
            );
        }
    }
    for message in &mut messages {
        ensure!(message.is_object(), "message must be an object");
        let assistant = message["role"] == "assistant";
        if let Some(calls) = message.get_mut("tool_calls").filter(|v| !v.is_null()) {
            let calls = calls
                .as_array_mut()
                .context("tool_calls must be an array")?;
            ensure!(
                calls.is_empty() || assistant,
                "tool calls require assistant role"
            );
            for call in calls {
                ensure!(call["type"] == "function", "unsupported tool call type");
                let function = call
                    .get_mut("function")
                    .and_then(Value::as_object_mut)
                    .context("tool function object required")?;
                ensure!(
                    function
                        .get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|v| !v.is_empty()),
                    "tool function name required"
                );
                let arguments = function
                    .get_mut("arguments")
                    .context("tool arguments required")?;
                if let Some(text) = arguments.as_str() {
                    *arguments = serde_json::from_str(text)
                        .map_err(|_| anyhow::anyhow!("invalid serialized tool arguments"))?;
                }
                ensure!(
                    arguments.is_object(),
                    "tool arguments must be a JSON object"
                );
            }
        }
    }
    Ok(Value::Array(messages))
}

/// Render the exact final body that the caller will issue. No network, loaders,
/// remote scripts, system commands or tool execution are available to templates.
/// Unsupported template features fail rather than silently substituting framing.
pub fn render_wire_body(
    body_bytes: &[u8],
    tokenizer_path: &Path,
    template_path: &Path,
    identity: &RenderingIdentity,
    special_tokens: &Value,
    limits: RenderLimits,
) -> Result<RenderedChat> {
    ensure!(
        identity.renderer_version == RENDERER_VERSION && !identity.tokenizer_id.is_empty(),
        "rendering identity does not name this renderer/tokenizer"
    );
    ensure!(
        body_bytes.len() <= limits.max_body_bytes,
        "wire body byte budget exceeded"
    );
    ensure!(
        identity.tokenizer_revision.len() == 40
            && identity
                .tokenizer_revision
                .bytes()
                .all(|b| b.is_ascii_hexdigit()),
        "tokenizer revision must be pinned"
    );
    let tokenizer_bytes = artifact(tokenizer_path, limits.max_artifact_bytes)?;
    let template_bytes = artifact(template_path, limits.max_artifact_bytes)?;
    ensure!(
        sha256(&tokenizer_bytes) == identity.tokenizer_sha256,
        "tokenizer checksum mismatch"
    );
    ensure!(
        sha256(&template_bytes) == identity.chat_template_sha256,
        "template checksum mismatch"
    );
    let body: Value = serde_json::from_slice(body_bytes).context("invalid final HTTP JSON body")?;
    ensure!(
        body["messages"]
            .as_array()
            .is_some_and(|messages| !messages.is_empty()),
        "nonempty messages required"
    );
    let mut context: std::collections::BTreeMap<String, minijinja::Value> = special_tokens
        .as_object()
        .context("special-token context must be an object")?
        .iter()
        .map(|(key, value)| (key.clone(), minijinja::Value::from(Serde(value))))
        .collect();
    for reserved in ["messages", "tools", "documents", "add_generation_prompt"] {
        ensure!(
            !context.contains_key(reserved),
            "special-token context cannot replace reserved input {reserved}"
        );
    }
    template_messages(&body)?; // Validate shapes without losing wire-object order.
    let ordered: minijinja::Value = serde_json::from_slice(body_bytes)
        .map_err(|_| anyhow::anyhow!("invalid ordered template body"))?;
    context.insert("messages".into(), template_context::messages(&ordered)?);
    for field in ["tools", "documents"] {
        let value = ordered.get_attr(field)?;
        context.insert(
            field.into(),
            if value.is_undefined() {
                minijinja::Value::from(())
            } else {
                value
            },
        );
    }
    context.insert(
        "add_generation_prompt".into(),
        minijinja::Value::from(identity.add_generation_prompt),
    );
    let template = std::str::from_utf8(&template_bytes).context("template must be UTF-8")?;
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.set_fuel(Some(limits.template_fuel));
    env.set_recursion_limit(64);
    env.add_filter(
        "tojson",
        move |value: minijinja::Value, args: minijinja::value::Kwargs| {
            template_context::tojson(value, args, limits.max_rendered_bytes)
        },
    );
    env.add_function(
        "raise_exception",
        |_message: String| -> std::result::Result<String, Error> {
            Err(Error::new(
                ErrorKind::InvalidOperation,
                "chat template rejected input",
            ))
        },
    );
    env.add_template("chat", template)
        .map_err(|e| anyhow::anyhow!("invalid chat template: {}", e.kind()))?;
    let mut output = BoundedOutput {
        bytes: Vec::new(),
        max: limits.max_rendered_bytes,
    };
    env.get_template("chat")?
        .render_captured_to(Serde(&context), &mut output)
        .map_err(|e| anyhow::anyhow!("chat rendering failed: {}", e.kind()))?;
    let text = std::str::from_utf8(&output.bytes).context("invalid rendered UTF-8")?;
    let tokenizer = tokenizers::Tokenizer::from_bytes(&tokenizer_bytes)
        .map_err(|_| anyhow::anyhow!("invalid pinned tokenizer artifact"))?;
    ensure!(
        tokenizer.get_truncation().is_none() && tokenizer.get_padding().is_none(),
        "implicit tokenizer truncation/padding is unsupported"
    );
    let encoded = tokenizer
        .encode(text, identity.add_special_tokens)
        .map_err(|_| anyhow::anyhow!("rendered chat tokenization failed"))?;
    ensure!(
        encoded.len() <= limits.max_tokens,
        "rendered token budget exceeded"
    );
    Ok(RenderedChat {
        body_sha256: sha256(body_bytes),
        rendered_text_sha256: sha256(&output.bytes),
        token_ids: encoded.get_ids().to_vec(),
        identity: identity.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn tools_calls_results_and_generation_prompt_have_golden_tokens() {
        let template = b"{{ tools[0].function.name }}|{{ messages[1].tool_calls[0].function.arguments.city }}|{{ messages[2].tool_call_id }}|{{ messages[2].content }}{% if add_generation_prompt %}|GEN{% endif %}";
        let expected = "lookup|Paris|call1|inert";
        let tokenizer = serde_json::to_vec(&json!({"version":"1.0","truncation":null,"padding":null,"added_tokens":[],"normalizer":null,"pre_tokenizer":null,"post_processor":null,"decoder":null,"model":{"type":"WordLevel","vocab":{expected:0,format!("{expected}|GEN"):1,"[UNK]":2},"unk_token":"[UNK]"}})).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let tok = dir.path().join("tokenizer.json");
        let tpl = dir.path().join("chat.jinja");
        std::fs::write(&tok, &tokenizer).unwrap();
        std::fs::write(&tpl, template).unwrap();
        let mut identity = RenderingIdentity {
            tokenizer_id: "fixture/wordlevel".into(),
            tokenizer_revision: "a".repeat(40),
            tokenizer_sha256: sha256(&tokenizer),
            chat_template_sha256: sha256(template),
            add_special_tokens: false,
            add_generation_prompt: false,
            hint_nonce_included: false,
            renderer_version: RENDERER_VERSION.into(),
        };
        let limits = RenderLimits {
            max_artifact_bytes: 8192,
            max_body_bytes: 8192,
            max_rendered_bytes: 512,
            max_tokens: 8,
            template_fuel: 10000,
        };
        let mut body = json!({"tools":[{"type":"function","function":{"name":"lookup","parameters":{"type":"object"}}}],"messages":[{"role":"user","content":"question"},{"role":"assistant","content":null,"tool_calls":[{"id":"call1","type":"function","function":{"name":"lookup","arguments":"{\"city\":\"Paris\"}"}}]},{"role":"tool","tool_call_id":"call1","content":"inert"}]});
        for generation in [false, true] {
            identity.add_generation_prompt = generation;
            let wire = serde_json::to_vec(&body).unwrap();
            let rendered =
                render_wire_body(&wire, &tok, &tpl, &identity, &json!({}), limits).unwrap();
            assert_eq!(rendered.token_ids, vec![u32::from(generation)]);
            assert_eq!(rendered.body_sha256, sha256(&wire));
            assert_eq!(
                rendered.rendered_text_sha256,
                sha256(
                    if generation {
                        "lookup|Paris|call1|inert|GEN"
                    } else {
                        expected
                    }
                    .as_bytes()
                )
            );
        }
        body["messages"][1]["tool_calls"][0]["function"]["arguments"] = json!({"city":"Paris"});
        assert_eq!(
            render_wire_body(
                &serde_json::to_vec(&body).unwrap(),
                &tok,
                &tpl,
                &identity,
                &json!({}),
                limits
            )
            .unwrap()
            .token_ids,
            vec![1]
        );
        for invalid in [json!("SECRET_NOT_JSON"), json!("[]"), json!(null)] {
            body["messages"][1]["tool_calls"][0]["function"]["arguments"] = invalid;
            let error = render_wire_body(
                &serde_json::to_vec(&body).unwrap(),
                &tok,
                &tpl,
                &identity,
                &json!({}),
                limits,
            )
            .unwrap_err()
            .to_string();
            assert!(!error.contains("SECRET_NOT_JSON"));
        }
        assert!(
            template_messages(&json!({"messages":[{"role":"user","tool_calls":[{}]}]})).is_err()
        );
        assert!(template_messages(&json!({"messages":[{}],"tools":{}})).is_err());
    }

    #[test]
    fn full_wire_context_keeps_hf_json_order_and_no_html_escaping() {
        let template = b"{{ tools[0].function.parameters|tojson }}|{{ messages[1].tool_calls[0].function.arguments|tojson }}";
        let expected = "{\"z\": \"<é>\", \"a\": 1}|{\"y\": true, \"b\": [2, 1]}";
        let tokenizer = serde_json::to_vec(&json!({"version":"1.0","truncation":null,"padding":null,"added_tokens":[],"normalizer":null,"pre_tokenizer":null,"post_processor":null,"decoder":null,"model":{"type":"WordLevel","vocab":{expected:0,"[UNK]":1},"unk_token":"[UNK]"}})).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let tok = dir.path().join("tokenizer.json");
        let tpl = dir.path().join("chat.jinja");
        std::fs::write(&tok, &tokenizer).unwrap();
        std::fs::write(&tpl, template).unwrap();
        let identity = RenderingIdentity {
            tokenizer_id: "fixture/ordered".into(),
            tokenizer_revision: "a".repeat(40),
            tokenizer_sha256: sha256(&tokenizer),
            chat_template_sha256: sha256(template),
            add_special_tokens: false,
            add_generation_prompt: false,
            hint_nonce_included: false,
            renderer_version: RENDERER_VERSION.into(),
        };
        let wire = r#"{"tools":[{"type":"function","function":{"name":"inert","parameters":{"z":"<é>","a":1}}}],"messages":[{"role":"user","content":"question"},{"role":"assistant","tool_calls":[{"type":"function","function":{"name":"inert","arguments":"{\"y\":true,\"b\":[2,1]}"}}]}]}"#.as_bytes();
        let rendered = render_wire_body(
            wire,
            &tok,
            &tpl,
            &identity,
            &json!({}),
            RenderLimits {
                max_artifact_bytes: 8192,
                max_body_bytes: 8192,
                max_rendered_bytes: 1024,
                max_tokens: 8,
                template_fuel: 10000,
            },
        )
        .unwrap();
        assert_eq!(rendered.token_ids, vec![0]);
        assert_eq!(rendered.rendered_text_sha256, sha256(expected.as_bytes()));
        assert_eq!(rendered.body_sha256, sha256(wire));
    }

    #[test]
    fn pinned_template_renders_actual_wire_body_and_keeps_hash_bases_separate() {
        let tokenizer = br#"{"version":"1.0","truncation":null,"padding":null,"added_tokens":[],"normalizer":null,"pre_tokenizer":null,"post_processor":null,"decoder":null,"model":{"type":"BPE","dropout":null,"unk_token":null,"continuing_subword_prefix":null,"end_of_word_suffix":null,"fuse_unk":false,"byte_fallback":false,"vocab":{"a":0,"b":1,"ab":2},"merges":[["a","b"]]}}"#;
        let template = b"{% for message in messages %}{{ message.content }}{% endfor %}";
        let dir = tempfile::tempdir().unwrap();
        let tok = dir.path().join("tokenizer.json");
        let tpl = dir.path().join("chat.jinja");
        std::fs::write(&tok, tokenizer).unwrap();
        std::fs::write(&tpl, template).unwrap();
        let identity = RenderingIdentity {
            tokenizer_id: "fixture/bpe".into(),
            tokenizer_revision: "a".repeat(40),
            tokenizer_sha256: sha256(tokenizer),
            chat_template_sha256: sha256(template),
            add_special_tokens: false,
            add_generation_prompt: true,
            hint_nonce_included: false,
            renderer_version: RENDERER_VERSION.into(),
        };
        let limits = RenderLimits {
            max_artifact_bytes: 8192,
            max_body_bytes: 8192,
            max_rendered_bytes: 128,
            max_tokens: 128,
            template_fuel: 10000,
        };
        let body = br#"{"messages":[{"role":"user","content":"ab"}],"temperature":0}"#;
        let rendered = render_wire_body(body, &tok, &tpl, &identity, &json!({}), limits).unwrap();
        assert_eq!(rendered.token_ids, vec![2]);
        assert_eq!(rendered.body_sha256, sha256(body));
        assert_eq!(rendered.rendered_text_sha256, sha256(b"ab"));
        assert_ne!(rendered.body_sha256, rendered.rendered_text_sha256);
        let mut changed = identity.clone();
        changed.chat_template_sha256 = "f".repeat(64);
        assert!(render_wire_body(body, &tok, &tpl, &changed, &json!({}), limits).is_err());
        assert!(render_wire_body(
            body,
            &tok,
            &tpl,
            &identity,
            &json!({"messages": []}),
            limits
        )
        .is_err());
        assert!(render_wire_body(
            body,
            &tok,
            &tpl,
            &identity,
            &json!({}),
            RenderLimits {
                max_rendered_bytes: 1,
                ..limits
            }
        )
        .is_err());
        assert!(render_wire_body(
            body,
            &tok,
            &tpl,
            &identity,
            &json!({}),
            RenderLimits {
                max_tokens: 0,
                ..limits
            }
        )
        .is_err());
        std::fs::write(&tpl, b"{{ unsupported_function() }}").unwrap();
        let mut changed = identity;
        changed.chat_template_sha256 = sha256(b"{{ unsupported_function() }}");
        assert!(render_wire_body(body, &tok, &tpl, &changed, &json!({}), limits).is_err());
    }
}
