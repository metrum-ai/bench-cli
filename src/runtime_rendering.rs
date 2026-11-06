// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Runtime consumption of the producer-owned pinned renderer. No HTTP or template
//! implementation here; callers must issue the retained exact bytes unchanged.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::chat_rendering::{render_wire_body, RenderLimits, RenderedChat};
use crate::prefix_validation::RenderingIdentity;
use crate::token_overlap::{Phase, RenderedBody};

/// Local, operator-selected artifacts/context, never dataset-provided executable
/// templates. The producer checks artifact hashes, budgets and formatting policy.
pub struct PinnedRenderer<'a> {
    pub tokenizer_path: &'a Path,
    pub template_path: &'a Path,
    pub identity: &'a RenderingIdentity,
    pub special_tokens: &'a Value,
    pub limits: RenderLimits,
    /// A requested comparison must complete before tokens become eligible.
    pub require_server_cross_check: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenizeCrossCheck {
    NotRequested,
    Pending,
    Match,
    Mismatch,
    Unavailable,
}

/// Manifest-safe fingerprints/settings only: no bodies, tool arguments, special
/// token bindings or token IDs. This is logical rendering, not physical KV proof.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireRenderingEvidence {
    pub schema_version: String,
    pub body_sha256: String,
    pub rendered_text_sha256: String,
    pub identity: RenderingIdentity,
    pub server_tokenize_cross_check: TokenizeCrossCheck,
}

/// Intentionally neither Debug nor Serialize. Own immutable final bytes, so the
/// HTTP caller can use `.body(prepared.wire_bytes().to_vec())`, not reserialize a
/// JSON value or apply later history/nonces/cache transformations.
pub struct PreparedWireBody {
    wire: Vec<u8>,
    rendered: RenderedBody,
    evidence: WireRenderingEvidence,
}

impl PreparedWireBody {
    pub fn wire_bytes(&self) -> &[u8] {
        &self.wire
    }
    pub fn evidence(&self) -> &WireRenderingEvidence {
        &self.evidence
    }

    /// The engine adapter must supply exact integer IDs for this same body and
    /// pinned formatting policy. None means requested but unavailable/invalid.
    /// Failure is latched: a later success cannot erase contradictory evidence.
    pub fn record_server_token_ids(&mut self, ids: Option<&[u32]>) {
        use TokenizeCrossCheck::*;
        if matches!(
            self.evidence.server_tokenize_cross_check,
            Mismatch | Unavailable
        ) {
            return;
        }
        self.evidence.server_tokenize_cross_check = match ids {
            None => Unavailable,
            Some(ids) if ids == self.rendered.token_ids => Match,
            Some(_) => Mismatch,
        };
    }

    /// Requested cross-check failure invalidates the rendering evidence fed into
    /// the authoritative validator. Empty tokens force Unknown, never Pass/Fail.
    /// The actual offline tokens remain private for cross-checking, not exported.
    pub fn for_overlap(&self) -> RenderedBody {
        let mut body = self.rendered.clone();
        if matches!(
            self.evidence.server_tokenize_cross_check,
            TokenizeCrossCheck::Pending
                | TokenizeCrossCheck::Mismatch
                | TokenizeCrossCheck::Unavailable
        ) {
            body.token_ids.clear();
        }
        body
    }
}

/// Consume exact HTTP bytes after ALL transformations. No fallback renderer,
/// reserialization, padding, truncation or inferred tokenizer identity.
pub fn prepare_wire_body(
    request_id: String,
    phase: Phase,
    wire: Vec<u8>,
    config: &PinnedRenderer<'_>,
) -> Result<PreparedWireBody> {
    let mut body_sha256 = String::with_capacity(64);
    for byte in Sha256::digest(&wire) {
        write!(&mut body_sha256, "{byte:02x}").expect("format SHA-256 into String");
    }
    let RenderedChat {
        body_sha256: producer_digest,
        rendered_text_sha256,
        token_ids,
        identity,
    } = render_wire_body(
        &wire,
        config.tokenizer_path,
        config.template_path,
        config.identity,
        config.special_tokens,
        config.limits,
    )
    .map_err(|_| anyhow::anyhow!("pinned renderer rejected final request body"))?;
    ensure!(
        producer_digest == body_sha256 && identity == *config.identity,
        "pinned renderer wire/identity contract mismatch"
    );
    ensure!(
        !token_ids.is_empty(),
        "empty rendered chat tokens cannot establish isolation"
    );
    let rendered = RenderedBody {
        request_id,
        phase,
        body_sha256: body_sha256.clone(),
        tokenizer_revision: identity.tokenizer_revision.clone(),
        chat_template_sha256: identity.chat_template_sha256.clone(),
        token_ids,
    };
    Ok(PreparedWireBody {
        wire,
        rendered,
        evidence: WireRenderingEvidence {
            schema_version: "kv-wire-rendering.v1".into(),
            body_sha256,
            rendered_text_sha256,
            identity,
            server_tokenize_cross_check: if config.require_server_cross_check {
                TokenizeCrossCheck::Pending
            } else {
                TokenizeCrossCheck::NotRequested
            },
        },
    })
}
