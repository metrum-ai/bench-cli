// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Exact rendered-token overlap, not server hit-rate evidence.

use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Full post-template token IDs and immutable record/group identity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderedRecord {
    pub record_id: String,
    pub group_id: Option<String>,
    pub tokens: Vec<u32>,
    pub rendered_body_sha256: String,
    pub supplied_input_target: Option<u64>,
}

/// Artifact identity is part of all validation fingerprints.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderingIdentity {
    pub tokenizer_id: String,
    pub tokenizer_revision: String,
    pub tokenizer_sha256: String,
    pub chat_template_sha256: String,
    pub add_special_tokens: bool,
    pub add_generation_prompt: bool,
    pub hint_nonce_included: bool,
    pub renderer_version: String,
}

/// Encode an already fully rendered body with a pinned local tokenizer. The
/// caller renders the exact template artifact and supplies all hints/nonces before
/// this boundary. No template or tokenizer is downloaded or executed remotely.
#[cfg(feature = "tokenizer")]
pub fn tokenize_rendered(
    record_id: String,
    group_id: Option<String>,
    rendered_body: &str,
    tokenizer_path: &std::path::Path,
    template_bytes: &[u8],
    identity: &RenderingIdentity,
    supplied_input_target: Option<u64>,
) -> Result<RenderedRecord> {
    let bytes = std::fs::read(tokenizer_path)?;
    ensure!(
        crate::public_dataset::sha256(&bytes) == identity.tokenizer_sha256,
        "tokenizer checksum mismatch"
    );
    ensure!(
        crate::public_dataset::sha256(template_bytes) == identity.chat_template_sha256,
        "template checksum mismatch"
    );
    let tokenizer = tokenizers::Tokenizer::from_bytes(&bytes)
        .map_err(|e| anyhow::anyhow!("load pinned tokenizer: {e}"))?;
    let encoded = tokenizer
        .encode(rendered_body, identity.add_special_tokens)
        .map_err(|e| anyhow::anyhow!("encode rendered input: {e}"))?;
    Ok(RenderedRecord {
        record_id,
        group_id,
        tokens: encoded.get_ids().to_vec(),
        rendered_body_sha256: crate::public_dataset::sha256(rendered_body.as_bytes()),
        supplied_input_target,
    })
}

/// Opt-in constraints. Shared template roots count toward the LCP threshold.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SelectionConstraints {
    pub content_unique: bool,
    pub max_pair_lcp: Option<usize>,
}

/// Exact distribution (nearest-rank percentiles); sample-scoped, never corpus certification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Distribution {
    pub n: usize,
    pub min: Option<usize>,
    pub p50: Option<usize>,
    pub p95: Option<usize>,
    pub max: Option<usize>,
}

impl Distribution {
    fn from_values(mut values: Vec<usize>) -> Self {
        values.sort_unstable();
        let n = values.len();
        let rank = |percent: usize| {
            values
                .get((n * percent).div_ceil(100).saturating_sub(1))
                .copied()
        };
        Self {
            n,
            min: values.first().copied(),
            p50: rank(50),
            p95: rank(95),
            max: values.last().copied(),
        }
    }
}

/// Exhaustive bounded pairwise validation, including held-out warmup comparisons.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OverlapValidation {
    pub schema_version: u32,
    pub identity: RenderingIdentity,
    pub sample_size: usize,
    pub lengths: Distribution,
    pub within_group_lcp: Distribution,
    pub across_group_lcp: Distribution,
    pub warmup_lcp: Distribution,
    pub block_size: usize,
    pub block_aligned_reuse_ceiling: Distribution,
    pub exact_token_duplicates: usize,
    pub exact_body_duplicates: usize,
    pub body_order_sha256: String,
    pub limitations: Vec<String>,
}

/// Number of exactly matching tokens from the beginning, including chat framing.
pub fn lcp(a: &[u32], b: &[u32]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

/// Deterministic first-fit subset preserving candidate order. Impossible requests fail
/// before traffic. Without constraints this is the original first-N order.
pub fn select(
    candidates: &[RenderedRecord],
    count: usize,
    constraints: SelectionConstraints,
) -> Result<Vec<RenderedRecord>> {
    let mut selected: Vec<RenderedRecord> = Vec::new();
    if count == 0 {
        return Ok(selected);
    }
    for candidate in candidates {
        if selected.iter().any(|prior| {
            (constraints.content_unique
                && (prior.tokens == candidate.tokens
                    || prior.rendered_body_sha256 == candidate.rendered_body_sha256))
                || constraints
                    .max_pair_lcp
                    .is_some_and(|max| lcp(&prior.tokens, &candidate.tokens) > max)
        }) {
            continue;
        }
        selected.push(candidate.clone());
        if selected.len() == count {
            return Ok(selected);
        }
    }
    anyhow::bail!(
        "constrained selection impossible: requested {count}, achieved {} from {} candidates",
        selected.len(),
        candidates.len()
    )
}

/// Validate bounded, fully rendered input. IDs or supplied targets never stand in
/// for token identity. max_pairs bounds quadratic work explicitly; no pair sampling.
pub fn validate(
    records: &[RenderedRecord],
    warmup: &[RenderedRecord],
    identity: RenderingIdentity,
    block_size: usize,
    max_pairs: usize,
    warmup_max_lcp: Option<usize>,
) -> Result<OverlapValidation> {
    ensure!(block_size > 0, "block size must be positive");
    ensure!(
        !identity.tokenizer_id.is_empty() && !identity.renderer_version.is_empty(),
        "render identity required"
    );
    let is_hex = |s: &str, n| s.len() == n && s.bytes().all(|b| b.is_ascii_hexdigit());
    ensure!(
        is_hex(&identity.tokenizer_revision, 40),
        "tokenizer revision must be pinned"
    );
    ensure!(
        is_hex(&identity.tokenizer_sha256, 64) && is_hex(&identity.chat_template_sha256, 64),
        "render artifact digests required"
    );
    let pairs = records
        .len()
        .checked_mul(records.len().saturating_sub(1))
        .and_then(|n| n.checked_div(2))
        .and_then(|n| {
            records
                .len()
                .checked_mul(warmup.len())
                .and_then(|w| n.checked_add(w))
        });
    ensure!(
        pairs.is_some_and(|n| n <= max_pairs),
        "pair validation budget exceeded"
    );
    let mut ids = BTreeSet::new();
    for r in records {
        ensure!(
            !r.record_id.is_empty() && ids.insert(&r.record_id),
            "duplicate or empty record ID"
        );
        ensure!(
            !r.tokens.is_empty() && is_hex(&r.rendered_body_sha256, 64),
            "invalid rendered record"
        );
    }
    let mut within = Vec::new();
    let mut across = Vec::new();
    let mut blocks = Vec::new();
    let mut warm = Vec::new();
    let mut duplicate_tokens = 0;
    let mut duplicate_bodies = 0;
    for (i, r) in records.iter().enumerate() {
        let mut best = 0;
        for prior in &records[..i] {
            let overlap = lcp(&r.tokens, &prior.tokens);
            best = best.max(overlap);
            if r.group_id.is_some() && r.group_id == prior.group_id {
                within.push(overlap);
            } else {
                across.push(overlap);
            }
            duplicate_tokens += usize::from(r.tokens == prior.tokens);
            duplicate_bodies += usize::from(r.rendered_body_sha256 == prior.rendered_body_sha256);
        }
        // Conservative exact full-block ceiling; partial final blocks never count.
        blocks.push(best / block_size * block_size);
        for w in warmup {
            let overlap = lcp(&r.tokens, &w.tokens);
            ensure!(
                warmup_max_lcp.is_none_or(|max| overlap <= max),
                "warmup prefix-disjoint validation failed for {}",
                r.record_id
            );
            warm.push(overlap);
        }
    }
    Ok(OverlapValidation {
        schema_version: 1, identity, sample_size: records.len(),
        lengths: Distribution::from_values(records.iter().map(|r| r.tokens.len()).collect()),
        within_group_lcp: Distribution::from_values(within), across_group_lcp: Distribution::from_values(across),
        warmup_lcp: Distribution::from_values(warm), block_size,
        block_aligned_reuse_ceiling: Distribution::from_values(blocks),
        exact_token_duplicates: duplicate_tokens, exact_body_duplicates: duplicate_bodies,
        body_order_sha256: crate::public_dataset::sha256(&serde_json::to_vec(records)?),
        limitations: vec!["Sample-only exhaustive pairwise logical overlap; not a corpus certificate or observed server hit rate. Reuse ceilings assume all prior prefixes resident, no eviction, and ignore warmup residency.".into()],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(id: &str, group: &str, tokens: &[u32]) -> RenderedRecord {
        RenderedRecord {
            record_id: id.into(),
            group_id: Some(group.into()),
            tokens: tokens.into(),
            rendered_body_sha256: crate::public_dataset::sha256(
                &serde_json::to_vec(tokens).unwrap(),
            ),
            supplied_input_target: Some(999),
        }
    }
    fn identity() -> RenderingIdentity {
        RenderingIdentity {
            tokenizer_id: "test/wordlevel".into(),
            tokenizer_revision: "a".repeat(40),
            tokenizer_sha256: "b".repeat(64),
            chat_template_sha256: "c".repeat(64),
            add_special_tokens: false,
            add_generation_prompt: true,
            hint_nonce_included: false,
            renderer_version: "fixture-v1".into(),
        }
    }
    #[cfg(feature = "tokenizer")]
    #[test]
    fn local_bpe_detects_boundary_merges_and_checks_artifact_pins() {
        let tokenizer_bytes = br#"{"version":"1.0","truncation":null,"padding":null,"added_tokens":[],"normalizer":null,"pre_tokenizer":null,"post_processor":null,"decoder":null,"model":{"type":"BPE","dropout":null,"unk_token":null,"continuing_subword_prefix":null,"end_of_word_suffix":null,"fuse_unk":false,"byte_fallback":false,"vocab":{"a":0,"b":1,"ab":2,"c":3},"merges":[["a","b"]]}}"#;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tokenizer.json");
        std::fs::write(&path, tokenizer_bytes).unwrap();
        let mut pin = identity();
        pin.tokenizer_sha256 = crate::public_dataset::sha256(tokenizer_bytes);
        pin.chat_template_sha256 = crate::public_dataset::sha256(b"fixture-framing-v1");
        let a = tokenize_rendered(
            "a".into(),
            None,
            "ab",
            &path,
            b"fixture-framing-v1",
            &pin,
            Some(100),
        )
        .unwrap();
        let b = tokenize_rendered(
            "b".into(),
            None,
            "ac",
            &path,
            b"fixture-framing-v1",
            &pin,
            Some(100),
        )
        .unwrap();
        assert_eq!(a.tokens, vec![2]);
        assert_eq!(b.tokens, vec![0, 3]);
        // Shared source character 'a' does not imply a shared rendered token.
        assert_eq!(lcp(&a.tokens, &b.tokens), 0);
        assert_ne!(a.tokens.len() as u64, a.supplied_input_target.unwrap());
        assert!(tokenize_rendered(
            "bad".into(),
            None,
            "ab",
            &path,
            b"changed-template",
            &pin,
            None
        )
        .is_err());
        pin.tokenizer_sha256 = "f".repeat(64);
        assert!(tokenize_rendered(
            "bad".into(),
            None,
            "ab",
            &path,
            b"fixture-framing-v1",
            &pin,
            None
        )
        .is_err());
    }

    #[test]
    fn distinct_rows_identical_content_are_not_unique() {
        let rows = vec![record("1", "g", &[1, 2]), record("2", "g", &[1, 2])];
        assert_eq!(
            select(&rows, 2, SelectionConstraints::default()).unwrap(),
            rows
        );
        assert!(select(
            &rows,
            2,
            SelectionConstraints {
                content_unique: true,
                max_pair_lcp: None
            }
        )
        .is_err());
        assert!(select(&rows, 0, SelectionConstraints::default())
            .unwrap()
            .is_empty());
    }
    #[test]
    fn prefix_constraints_and_partial_blocks() {
        let rows = vec![
            record("1", "g", &[1, 2, 3, 4, 5]),
            record("2", "g", &[1, 2, 3, 4, 6]),
            record("3", "h", &[1, 8]),
        ];
        assert!(select(
            &rows,
            3,
            SelectionConstraints {
                content_unique: true,
                max_pair_lcp: Some(1)
            }
        )
        .is_err());
        let selected = select(
            &rows,
            2,
            SelectionConstraints {
                content_unique: true,
                max_pair_lcp: Some(1),
            },
        )
        .unwrap();
        assert_eq!(selected[1].record_id, "3");
        let v = validate(&rows, &[], identity(), 3, 10, None).unwrap();
        assert_eq!(v.lengths.min, Some(2));
        assert_eq!(v.lengths.max, Some(5));
        assert_eq!(v.within_group_lcp.max, Some(4));
        assert_eq!(v.across_group_lcp.max, Some(1));
        assert_eq!(v.block_aligned_reuse_ceiling.max, Some(3));
        assert_eq!(v.exact_token_duplicates, 0);
    }
    #[test]
    fn warmup_pins_budgets_and_order() {
        let rows = vec![record("1", "g", &[1, 2]), record("2", "h", &[3, 4])];
        let warm = vec![record("w", "warm", &[1, 2, 3])];
        assert!(validate(&rows, &warm, identity(), 1, 10, Some(1)).is_err());
        assert!(validate(&rows, &warm, identity(), 1, 1, None).is_err());
        assert!(validate(&rows, &[], identity(), 0, 10, None).is_err());
        let v = validate(&rows, &warm, identity(), 1, 10, Some(2)).unwrap();
        assert_eq!(v.warmup_lcp.max, Some(2));
        let mut changed = identity();
        changed.tokenizer_revision = "main".into();
        assert!(validate(&rows, &[], changed, 1, 10, None).is_err());
        let reversed = vec![rows[1].clone(), rows[0].clone()];
        assert_ne!(
            v.body_order_sha256,
            validate(&reversed, &warm, identity(), 1, 10, None)
                .unwrap()
                .body_order_sha256
        );
        assert_eq!(Distribution::from_values(vec![]).p50, None);
    }
}
