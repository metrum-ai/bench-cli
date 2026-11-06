// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Pure offline warmup/measured rendered-token isolation interface.
//! HTTP body digests are supplied by the caller after every transformation.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Issuance phase of the exact rendered body.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Phase {
    Warmup,
    Measured,
}

/// Fully transformed wire body identity and its fully rendered chat tokens.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RenderedBody {
    pub request_id: String,
    pub phase: Phase,
    pub body_sha256: String,
    pub tokenizer_revision: String,
    pub chat_template_sha256: String,
    pub token_ids: Vec<u32>,
}

/// Unknown must never be treated as an isolation pass.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail,
    Unknown,
}

/// Logical block-prefix isolation, not evidence of physical cache state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OverlapReport {
    pub schema_version: String,
    pub warmup_body_digests_sha256: String,
    pub measured_body_digests_sha256: String,
    pub max_shared_prefix_tokens: usize,
    pub max_shared_full_blocks: usize,
    /// Warmup request ID, measured request ID, exact shared prefix token count.
    pub pairs_over_threshold: Vec<(String, String, usize)>,
    pub tokenizer_revision: String,
    pub chat_template_sha256: String,
    pub verdict: Verdict,
}

fn hex(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn body_digests_sha256(bodies: &[RenderedBody]) -> String {
    let mut digests: Vec<&str> = bodies.iter().map(|b| b.body_sha256.as_str()).collect();
    digests.sort_unstable();
    // Array/string serialization is infallible. Preserve duplicate body digests.
    let bytes = serde_json::to_vec(&digests).expect("serialize string digest list");
    let hash = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in hash {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

/// Default fail-closed boundary: no shared complete cache blocks are allowed.
/// Different file names or request IDs never establish isolation.
pub fn validate_prefix_disjoint(
    warmup: &[RenderedBody],
    measured: &[RenderedBody],
    block_size_tokens: usize,
) -> OverlapReport {
    validate_prefix_disjoint_with_threshold(warmup, measured, block_size_tokens, 0)
}

/// Configurable threshold in full blocks. Callers must record non-default policy.
/// Empty inputs, invalid pins/digests/phases/tokens, and mismatched rendering
/// identities return Unknown, never Pass. Maxima on Unknown are not measurements.
pub fn validate_prefix_disjoint_with_threshold(
    warmup: &[RenderedBody],
    measured: &[RenderedBody],
    block_size_tokens: usize,
    allowed_shared_full_blocks: usize,
) -> OverlapReport {
    let mut report = OverlapReport {
        schema_version: "kv-token-overlap.v1".into(),
        warmup_body_digests_sha256: body_digests_sha256(warmup),
        measured_body_digests_sha256: body_digests_sha256(measured),
        max_shared_prefix_tokens: 0,
        max_shared_full_blocks: 0,
        pairs_over_threshold: Vec::new(),
        tokenizer_revision: String::new(),
        chat_template_sha256: String::new(),
        verdict: Verdict::Unknown,
    };
    if block_size_tokens == 0 || warmup.is_empty() || measured.is_empty() {
        return report;
    }
    let first = &warmup[0];
    if !hex(&first.tokenizer_revision, 40) || !hex(&first.chat_template_sha256, 64) {
        return report;
    }
    for (bodies, phase) in [(warmup, Phase::Warmup), (measured, Phase::Measured)] {
        for body in bodies {
            if body.phase != phase
                || body.request_id.is_empty()
                || body.token_ids.is_empty()
                || !hex(&body.body_sha256, 64)
                || body.tokenizer_revision != first.tokenizer_revision
                || body.chat_template_sha256 != first.chat_template_sha256
            {
                return report;
            }
        }
    }
    report.tokenizer_revision = first.tokenizer_revision.clone();
    report.chat_template_sha256 = first.chat_template_sha256.clone();
    for a in warmup {
        for b in measured {
            let shared = a
                .token_ids
                .iter()
                .zip(&b.token_ids)
                .take_while(|(a, b)| a == b)
                .count();
            let blocks = shared / block_size_tokens;
            report.max_shared_prefix_tokens = report.max_shared_prefix_tokens.max(shared);
            report.max_shared_full_blocks = report.max_shared_full_blocks.max(blocks);
            if blocks > allowed_shared_full_blocks {
                report.pairs_over_threshold.push((
                    a.request_id.clone(),
                    b.request_id.clone(),
                    shared,
                ));
            }
        }
    }
    report.verdict = if report.max_shared_full_blocks <= allowed_shared_full_blocks {
        Verdict::Pass
    } else {
        Verdict::Fail
    };
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    fn body(id: &str, phase: Phase, tokens: &[u32]) -> RenderedBody {
        RenderedBody {
            request_id: id.into(),
            phase,
            body_sha256: if id == "w" {
                "a".repeat(64)
            } else {
                "b".repeat(64)
            },
            tokenizer_revision: "c".repeat(40),
            chat_template_sha256: "d".repeat(64),
            token_ids: tokens.into(),
        }
    }
    #[test]
    fn exact_interface_default_and_configurable_full_block_threshold() {
        let warm = vec![body("w", Phase::Warmup, &[1, 2, 3, 4, 5])];
        let measured = vec![body("different-file-id", Phase::Measured, &[1, 2, 3, 4, 9])];
        let report = validate_prefix_disjoint(&warm, &measured, 3);
        assert_eq!(report.verdict, Verdict::Fail);
        assert_eq!(report.max_shared_prefix_tokens, 4);
        assert_eq!(report.max_shared_full_blocks, 1);
        assert_eq!(
            report.pairs_over_threshold,
            vec![("w".into(), "different-file-id".into(), 4)]
        );
        assert_eq!(
            validate_prefix_disjoint_with_threshold(&warm, &measured, 3, 1).verdict,
            Verdict::Pass
        );
        assert_eq!(
            validate_prefix_disjoint(&warm, &measured, 5).verdict,
            Verdict::Pass
        );
    }
    #[test]
    fn mismatched_tokenizer_template_and_invalid_inputs_are_unknown() {
        let warm = vec![body("w", Phase::Warmup, &[1])];
        let base = body("m", Phase::Measured, &[2]);
        assert_eq!(
            validate_prefix_disjoint(&warm, std::slice::from_ref(&base), 1).verdict,
            Verdict::Pass
        );
        for field in 0..6 {
            let mut bad = base.clone();
            match field {
                0 => bad.tokenizer_revision = "e".repeat(40),
                1 => bad.chat_template_sha256 = "f".repeat(64),
                2 => bad.body_sha256 = "invalid".into(),
                3 => bad.phase = Phase::Warmup,
                4 => bad.token_ids.clear(),
                _ => bad.request_id.clear(),
            }
            assert_eq!(
                validate_prefix_disjoint(&warm, &[bad], 1).verdict,
                Verdict::Unknown
            );
        }
        assert_eq!(
            validate_prefix_disjoint(&warm, std::slice::from_ref(&base), 0).verdict,
            Verdict::Unknown
        );
        assert_eq!(
            validate_prefix_disjoint(&[], &[base], 1).verdict,
            Verdict::Unknown
        );
        assert_eq!(
            validate_prefix_disjoint(&warm, &[], 1).verdict,
            Verdict::Unknown
        );
    }
    #[test]
    fn sorted_digest_lists_preserve_multiplicity_and_bind_wire_body_changes() {
        let a = body("w", Phase::Warmup, &[1]);
        let b = body("other", Phase::Warmup, &[2]);
        assert_eq!(
            body_digests_sha256(&[a.clone(), b.clone()]),
            body_digests_sha256(&[b.clone(), a.clone()])
        );
        assert_ne!(
            body_digests_sha256(std::slice::from_ref(&a)),
            body_digests_sha256(&[a.clone(), a.clone()])
        );
        let mut changed = a.clone();
        changed.body_sha256 = "f".repeat(64);
        assert_ne!(body_digests_sha256(&[a]), body_digests_sha256(&[changed]));
    }
}
