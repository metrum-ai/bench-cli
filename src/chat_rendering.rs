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

/// Version of the local rendering algorithm (dependency versions are locked).
pub const RENDERER_VERSION: &str = "minijinja-chat.v1";

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
    let mut context = special_tokens
        .as_object()
        .context("special-token context must be an object")?
        .clone();
    for reserved in ["messages", "tools", "documents", "add_generation_prompt"] {
        ensure!(
            !context.contains_key(reserved),
            "special-token context cannot replace reserved input {reserved}"
        );
    }
    context.insert("messages".into(), body["messages"].clone());
    context.insert(
        "tools".into(),
        body.get("tools").cloned().unwrap_or(Value::Null),
    );
    context.insert(
        "documents".into(),
        body.get("documents").cloned().unwrap_or(Value::Null),
    );
    context.insert(
        "add_generation_prompt".into(),
        Value::Bool(identity.add_generation_prompt),
    );
    let template = std::str::from_utf8(&template_bytes).context("template must be UTF-8")?;
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.set_fuel(Some(limits.template_fuel));
    env.set_recursion_limit(64);
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
            renderer_version: "minijinja-chat.v1".into(),
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
