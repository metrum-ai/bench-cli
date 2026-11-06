// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Consumer projection of the authoritative offline token-overlap validator.
//! Runtime callers without a pinned final-chat renderer still receive Unknown.
//! Body fingerprints alone must never establish a Pass.

pub use crate::token_overlap::{Phase, RenderedBody, Verdict};
use serde::{Deserialize, Serialize};
use sha2::Digest;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlapReport {
    pub schema_version: String,
    pub validator_schema_version: String,
    pub warmup_body_digests_sha256: String,
    pub measured_body_digests_sha256: String,
    pub max_shared_prefix_tokens: usize,
    pub max_shared_full_blocks: usize,
    pub pairs_over_threshold: Vec<(String, String, usize)>,
    pub pairs_over_threshold_total: Option<u64>,
    pub pairs_over_threshold_truncated: bool,
    pub tokenizer_revision: String,
    pub chat_template_sha256: String,
    pub verdict: Verdict,
}

impl From<crate::token_overlap::OverlapReport> for OverlapReport {
    fn from(report: crate::token_overlap::OverlapReport) -> Self {
        // Exhaustive on purpose: the pending producer population-count/capping
        // amendment must require adapting this projection, not silently dropping
        // its new fields. Never derive a population total from the pair sample.
        let crate::token_overlap::OverlapReport {
            schema_version,
            warmup_body_digests_sha256,
            measured_body_digests_sha256,
            max_shared_prefix_tokens,
            max_shared_full_blocks,
            pairs_over_threshold,
            tokenizer_revision,
            chat_template_sha256,
            verdict,
        } = report;
        Self {
            schema_version: "kv-warmup-overlap.v1".into(),
            validator_schema_version: schema_version,
            warmup_body_digests_sha256,
            measured_body_digests_sha256,
            max_shared_prefix_tokens,
            max_shared_full_blocks,
            pairs_over_threshold,
            pairs_over_threshold_total: None,
            pairs_over_threshold_truncated: false,
            tokenizer_revision,
            chat_template_sha256,
            verdict,
        }
    }
}

/// SHA-256 of exact serialized request-body bytes (no raw content retained).
pub fn body_digest(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Delegate all token/pin/phase/overlap decisions to the producer-owned module.
/// The current producer API lacks population/truncation fields: retain an unknown
/// population total rather than guessing from its diagnostic list. Runtime token
/// rendering remains unavailable, so the CLI still cannot establish isolation.
pub fn validate_prefix_disjoint(
    warmup: &[RenderedBody],
    measured: &[RenderedBody],
    block_size_tokens: usize,
) -> OverlapReport {
    crate::token_overlap::validate_prefix_disjoint(warmup, measured, block_size_tokens).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn body(id: &str, phase: Phase, bytes: &[u8], tokens: Vec<u32>) -> RenderedBody {
        RenderedBody {
            request_id: id.into(),
            phase,
            body_sha256: body_digest(bytes),
            tokenizer_revision: "a".repeat(40),
            chat_template_sha256: "b".repeat(64),
            token_ids: tokens,
        }
    }

    #[test]
    fn projection_matches_authoritative_pass_fail_unknown_without_inventing_totals() {
        let warmup = body("w", Phase::Warmup, b"warmup wire", vec![1, 2, 3, 4]);
        for (measured, expected) in [
            (
                body("pass", Phase::Measured, b"different wire", vec![8, 9]),
                Verdict::Pass,
            ),
            (
                body("fail", Phase::Measured, b"same prefix wire", vec![1, 2, 3]),
                Verdict::Fail,
            ),
            (
                body("unknown", Phase::Measured, b"unrendered wire", vec![]),
                Verdict::Unknown,
            ),
        ] {
            let native = crate::token_overlap::validate_prefix_disjoint(
                std::slice::from_ref(&warmup),
                std::slice::from_ref(&measured),
                2,
            );
            let projected = validate_prefix_disjoint(std::slice::from_ref(&warmup), &[measured], 2);
            assert_eq!(native.verdict, expected);
            assert_eq!(projected.verdict, native.verdict);
            assert_eq!(projected.schema_version, "kv-warmup-overlap.v1");
            assert_eq!(projected.validator_schema_version, native.schema_version);
            assert_eq!(projected.pairs_over_threshold, native.pairs_over_threshold);
            assert_eq!(
                projected.max_shared_prefix_tokens,
                native.max_shared_prefix_tokens
            );
            assert_eq!(
                projected.max_shared_full_blocks,
                native.max_shared_full_blocks
            );
            assert_eq!(
                projected.warmup_body_digests_sha256,
                native.warmup_body_digests_sha256
            );
            assert_eq!(
                projected.measured_body_digests_sha256,
                native.measured_body_digests_sha256
            );
            assert_eq!(projected.tokenizer_revision, native.tokenizer_revision);
            assert_eq!(projected.chat_template_sha256, native.chat_template_sha256);
            let value = serde_json::to_value(&projected).unwrap();
            assert_eq!(
                value.get("pairs_over_threshold_total"),
                Some(&serde_json::Value::Null)
            );
            assert_eq!(value["pairs_over_threshold_truncated"], false);
        }
    }

    #[test]
    fn unrendered_runtime_bodies_stay_unknown_and_bind_sorted_body_multiplicity() {
        let a = body("a", Phase::Warmup, b"first", vec![]);
        let b = body("b", Phase::Warmup, b"second", vec![]);
        let report = validate_prefix_disjoint(&[a.clone(), b.clone()], &[], 16);
        assert_eq!(report.verdict, Verdict::Unknown);
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
