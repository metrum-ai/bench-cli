// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Temporary integration boundary for the offline rendered-token overlap validator.
//! Until the dataset validator and a pinned chat-token renderer are available,
//! this module MUST return Unknown. It must never infer a Pass from body hashes.

use serde::{Deserialize, Serialize};
use sha2::Digest;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Phase {
    Warmup,
    Measured,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderedBody {
    pub request_id: String,
    pub phase: Phase,
    pub body_sha256: String,
    pub tokenizer_revision: String,
    pub chat_template_sha256: String,
    pub token_ids: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    Pass,
    Fail,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlapReport {
    pub schema_version: String,
    pub warmup_body_digests_sha256: String,
    pub measured_body_digests_sha256: String,
    pub max_shared_prefix_tokens: usize,
    pub max_shared_full_blocks: usize,
    pub pairs_over_threshold: Vec<(String, String, usize)>,
    pub tokenizer_revision: String,
    pub chat_template_sha256: String,
    pub verdict: Verdict,
}

/// SHA-256 of exact serialized request-body bytes (no raw content retained).
pub fn body_digest(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn digest_list(bodies: &[RenderedBody]) -> String {
    let mut digests: Vec<&str> = bodies
        .iter()
        .map(|body| body.body_sha256.as_str())
        .collect();
    digests.sort_unstable();
    // JSON array serialization of string digests is infallible.
    let bytes = serde_json::to_vec(&digests).expect("serialize digest strings");
    body_digest(&bytes)
}

/// Local stub for the agreed dataset-validator API. Deliberately fail closed,
/// including for distinct bodies, matching pins, zero blocks or empty populations.
/// Replace with the dataset implementation; do not implement a competing algorithm.
pub fn validate_prefix_disjoint(
    warmup: &[RenderedBody],
    measured: &[RenderedBody],
    _block_size_tokens: usize,
) -> OverlapReport {
    OverlapReport {
        schema_version: "kv-warmup-overlap.stub.v1".into(),
        warmup_body_digests_sha256: digest_list(warmup),
        measured_body_digests_sha256: digest_list(measured),
        max_shared_prefix_tokens: 0,
        max_shared_full_blocks: 0,
        pairs_over_threshold: Vec::new(),
        tokenizer_revision: "unknown".into(),
        chat_template_sha256: "unknown".into(),
        verdict: Verdict::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(id: &str, bytes: &[u8]) -> RenderedBody {
        RenderedBody {
            request_id: id.into(),
            phase: Phase::Warmup,
            body_sha256: body_digest(bytes),
            tokenizer_revision: "fixture".into(),
            chat_template_sha256: "a".repeat(64),
            token_ids: vec![1, 2],
        }
    }

    #[test]
    fn local_stub_never_claims_isolation_and_digests_bind_sorted_bodies() {
        let a = body("a", b"first");
        let b = body("b", b"second");
        let report = validate_prefix_disjoint(&[a.clone(), b.clone()], &[], 16);
        assert_eq!(report.verdict, Verdict::Unknown);
        assert_eq!(report.max_shared_full_blocks, 0); // Unknown maxima are not measurements.
        let reordered = validate_prefix_disjoint(&[b.clone(), a.clone()], &[], 16);
        assert_eq!(
            report.warmup_body_digests_sha256,
            reordered.warmup_body_digests_sha256
        );
        let changed = validate_prefix_disjoint(&[a.clone(), a], &[b], 16);
        assert_ne!(
            report.warmup_body_digests_sha256,
            changed.warmup_body_digests_sha256
        );
        assert_ne!(
            report.measured_body_digests_sha256,
            changed.measured_body_digests_sha256
        );
        assert_eq!(
            validate_prefix_disjoint(&[], &[], 0).verdict,
            Verdict::Unknown
        );
    }
}
