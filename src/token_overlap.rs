// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Pure offline warmup/measured rendered-token isolation interface.
//! HTTP body digests are supplied by the caller after every transformation.

use std::{collections::BTreeMap, ops::Range};

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
    /// Exact pair count including duplicates; None when the verdict is Unknown.
    pub pairs_over_threshold_total: Option<u64>,
    pub pairs_over_threshold_truncated: bool,
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

/// Default bound on stored offending pairs; totals and maxima remain exact.
pub const DEFAULT_MAX_REPORTED_PAIRS: usize = 1000;

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
    validate_prefix_disjoint_with_limits(
        warmup,
        measured,
        block_size_tokens,
        allowed_shared_full_blocks,
        DEFAULT_MAX_REPORTED_PAIRS,
    )
}

/// Explicit storage cap, including zero. A cap never changes the verdict/maxima.
/// Stored pairs follow measured input order then warmup token-lexicographic order
/// (original warmup position breaks ties). Duplicate bodies retain multiplicity.
pub fn validate_prefix_disjoint_with_limits(
    warmup: &[RenderedBody],
    measured: &[RenderedBody],
    block_size_tokens: usize,
    allowed_shared_full_blocks: usize,
    max_reported_pairs: usize,
) -> OverlapReport {
    let mut report = OverlapReport {
        schema_version: "kv-token-overlap.v1".into(),
        warmup_body_digests_sha256: body_digests_sha256(warmup),
        measured_body_digests_sha256: body_digests_sha256(measured),
        max_shared_prefix_tokens: 0,
        max_shared_full_blocks: 0,
        pairs_over_threshold: Vec::new(),
        pairs_over_threshold_total: None,
        pairs_over_threshold_truncated: false,
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
    let trie = TokenTrie::new(warmup);
    // Overflow here means no representable token vector can cross the threshold.
    let threshold = allowed_shared_full_blocks
        .checked_add(1)
        .and_then(|n| n.checked_mul(block_size_tokens));
    let mut total = 0u64;
    for b in measured {
        let (shared, offenders) = trie.query(warmup, &b.token_ids, threshold);
        report.max_shared_prefix_tokens = report.max_shared_prefix_tokens.max(shared);
        if let Some(range) = offenders {
            let Some(next_total) = u64::try_from(range.len())
                .ok()
                .and_then(|count| total.checked_add(count))
            else {
                report.max_shared_prefix_tokens = 0;
                report.pairs_over_threshold.clear();
                report.tokenizer_revision.clear();
                report.chat_template_sha256.clear();
                return report;
            };
            total = next_total;
            let remaining = max_reported_pairs.saturating_sub(report.pairs_over_threshold.len());
            for &index in trie.order[range].iter().take(remaining) {
                let a = &warmup[index];
                let pair_shared = a
                    .token_ids
                    .iter()
                    .zip(&b.token_ids)
                    .take_while(|(a, b)| a == b)
                    .count();
                report.pairs_over_threshold.push((
                    a.request_id.clone(),
                    b.request_id.clone(),
                    pair_shared,
                ));
            }
        }
    }
    report.max_shared_full_blocks = report.max_shared_prefix_tokens / block_size_tokens;
    report.pairs_over_threshold_total = Some(total);
    report.pairs_over_threshold_truncated =
        (report.pairs_over_threshold.len() as u128) < u128::from(total);
    report.verdict = if report.max_shared_full_blocks <= allowed_shared_full_blocks {
        Verdict::Pass
    } else {
        Verdict::Fail
    };
    report
}

/// Compressed edges borrow spans of warmup token vectors; at most O(W) nodes,
/// not one allocation per token. Descendant intervals count pairs without listing
/// them. Each measured body is walked once, plus LCP work for stored pairs only.
struct TokenTrie {
    nodes: Vec<TrieNode>,
    order: Vec<usize>,
}

struct TrieNode {
    owner: usize,
    edge: Range<usize>,
    ranks: Range<usize>,
    children: BTreeMap<u32, usize>,
}

impl TokenTrie {
    fn new(warmup: &[RenderedBody]) -> Self {
        let mut order: Vec<_> = (0..warmup.len()).collect();
        order.sort_unstable_by(|&a, &b| {
            warmup[a]
                .token_ids
                .cmp(&warmup[b].token_ids)
                .then(a.cmp(&b))
        });
        let mut trie = Self {
            nodes: vec![TrieNode {
                owner: 0,
                edge: 0..0,
                ranks: 0..0,
                children: BTreeMap::new(),
            }],
            order,
        };
        for rank in 0..trie.order.len() {
            let owner = trie.order[rank];
            let tokens = &warmup[owner].token_ids;
            let (mut parent, mut depth) = (0, 0);
            loop {
                trie.nodes[parent].ranks.end = rank + 1;
                if depth == tokens.len() {
                    break;
                }
                let key = tokens[depth];
                let Some(&child) = trie.nodes[parent].children.get(&key) else {
                    let leaf = trie.nodes.len();
                    trie.nodes.push(TrieNode {
                        owner,
                        edge: depth..tokens.len(),
                        ranks: rank..rank + 1,
                        children: BTreeMap::new(),
                    });
                    trie.nodes[parent].children.insert(key, leaf);
                    break;
                };
                let old = &trie.nodes[child];
                let edge = old.edge.clone();
                let old_owner = old.owner;
                let shared = warmup[old_owner].token_ids[edge.clone()]
                    .iter()
                    .zip(&tokens[depth..])
                    .take_while(|(a, b)| a == b)
                    .count();
                if shared == edge.len() {
                    parent = child;
                    depth += shared;
                    continue;
                }
                let branch = trie.nodes.len();
                let old_key = warmup[old_owner].token_ids[edge.start + shared];
                let old_start = trie.nodes[child].ranks.start;
                trie.nodes.push(TrieNode {
                    owner: old_owner,
                    edge: edge.start..edge.start + shared,
                    ranks: old_start..rank + 1,
                    children: BTreeMap::from([(old_key, child)]),
                });
                trie.nodes[child].edge.start += shared;
                trie.nodes[parent].children.insert(key, branch);
                depth += shared;
                if depth < tokens.len() {
                    let leaf = trie.nodes.len();
                    trie.nodes.push(TrieNode {
                        owner,
                        edge: depth..tokens.len(),
                        ranks: rank..rank + 1,
                        children: BTreeMap::new(),
                    });
                    trie.nodes[branch].children.insert(tokens[depth], leaf);
                }
                break;
            }
        }
        trie
    }

    fn query(
        &self,
        warmup: &[RenderedBody],
        tokens: &[u32],
        threshold: Option<usize>,
    ) -> (usize, Option<Range<usize>>) {
        let (mut node, mut depth, mut offenders) = (0, 0, None);
        while depth < tokens.len() {
            let Some(&child) = self.nodes[node].children.get(&tokens[depth]) else {
                break;
            };
            let next = &self.nodes[child];
            let shared = warmup[next.owner].token_ids[next.edge.clone()]
                .iter()
                .zip(&tokens[depth..])
                .take_while(|(a, b)| a == b)
                .count();
            depth += shared;
            if offenders.is_none() && threshold.is_some_and(|limit| depth >= limit) {
                offenders = Some(next.ranks.clone());
            }
            if shared < next.edge.len() {
                break;
            }
            node = child;
        }
        (depth, offenders)
    }
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
    fn brute(
        warm: &[RenderedBody],
        measured: &[RenderedBody],
        block: usize,
        allowed: usize,
    ) -> (usize, Vec<(String, String, usize)>) {
        let mut maximum = 0;
        let mut pairs = Vec::new();
        for a in warm {
            for b in measured {
                let shared = a
                    .token_ids
                    .iter()
                    .zip(&b.token_ids)
                    .take_while(|(a, b)| a == b)
                    .count();
                maximum = maximum.max(shared);
                if shared / block > allowed {
                    pairs.push((a.request_id.clone(), b.request_id.clone(), shared));
                }
            }
        }
        pairs.sort();
        (maximum, pairs)
    }

    #[test]
    fn compressed_trie_matches_brute_force_with_duplicates_and_prefixes() {
        let mut seed = 11u64;
        for case in 0..24 {
            let mut bodies = |phase, count| {
                (0..count)
                    .map(|index| {
                        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                        let length = 1 + (seed % 12) as usize;
                        let tokens: Vec<_> = (0..length)
                            .map(|_| {
                                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                                ((seed >> 32) % 3) as u32
                            })
                            .collect();
                        body(&format!("{case}-{index}"), phase, &tokens)
                    })
                    .collect::<Vec<_>>()
            };
            let mut warm = bodies(Phase::Warmup, 17);
            let measured = bodies(Phase::Measured, 19);
            warm.extend([
                body("prefix", Phase::Warmup, &[1]),
                body("long", Phase::Warmup, &[1, 2, 3]),
                body("split", Phase::Warmup, &[1, 2, 4]),
            ]);
            warm.push(warm[0].clone());
            for block in 1..=5 {
                for allowed in 0..=3 {
                    let (maximum, expected) = brute(&warm, &measured, block, allowed);
                    let report = validate_prefix_disjoint_with_limits(
                        &warm,
                        &measured,
                        block,
                        allowed,
                        usize::MAX,
                    );
                    let mut actual = report.pairs_over_threshold.clone();
                    actual.sort();
                    assert_eq!(actual, expected);
                    assert_eq!(report.max_shared_prefix_tokens, maximum);
                    assert_eq!(report.max_shared_full_blocks, maximum / block);
                    assert_eq!(
                        report.pairs_over_threshold_total,
                        Some(expected.len() as u64)
                    );
                    assert!(!report.pairs_over_threshold_truncated);
                    assert_eq!(
                        report.verdict,
                        if expected.is_empty() {
                            Verdict::Pass
                        } else {
                            Verdict::Fail
                        }
                    );
                }
            }
        }
    }

    #[test]
    fn capped_pairs_do_not_cap_counts_maxima_or_fail_verdict() {
        let warm = vec![
            body("a", Phase::Warmup, &[1, 2]),
            body("b", Phase::Warmup, &[1, 2, 3, 4]),
            body("b", Phase::Warmup, &[1, 2, 3, 4]),
        ];
        let measured = vec![body("m", Phase::Measured, &[1, 2, 3, 4]); 3];
        for cap in [0, 1, 4, 9, 1000] {
            let report = validate_prefix_disjoint_with_limits(&warm, &measured, 2, 0, cap);
            assert_eq!(report.verdict, Verdict::Fail);
            assert_eq!(report.max_shared_prefix_tokens, 4);
            assert_eq!(report.max_shared_full_blocks, 2);
            assert_eq!(report.pairs_over_threshold_total, Some(9));
            assert_eq!(report.pairs_over_threshold.len(), cap.min(9));
            assert_eq!(report.pairs_over_threshold_truncated, cap < 9);
            assert_eq!(
                report,
                validate_prefix_disjoint_with_limits(&warm, &measured, 2, 0, cap)
            );
        }
        let report = validate_prefix_disjoint_with_limits(&warm, &measured, 1, usize::MAX, 0);
        assert_eq!(report.verdict, Verdict::Pass);
        assert_eq!(report.pairs_over_threshold_total, Some(0));
        assert!(!report.pairs_over_threshold_truncated);
        let unknown = validate_prefix_disjoint_with_limits(&[], &measured, 2, 0, 0);
        assert_eq!(unknown.verdict, Verdict::Unknown);
        assert_eq!(unknown.pairs_over_threshold_total, None);
        assert!(!unknown.pairs_over_threshold_truncated);
        let invalid_pin = vec![RenderedBody {
            tokenizer_revision: "bad".into(),
            ..warm[0].clone()
        }];
        assert_eq!(
            validate_prefix_disjoint(&invalid_pin, &measured, 2).pairs_over_threshold_total,
            None
        );
    }

    #[test]
    fn bounded_large_study_2k_by_2k_by_8k_tokens() {
        let warm: Vec<_> = (0..2000)
            .map(|i| body(&format!("w{i}"), Phase::Warmup, &vec![7; 8000]))
            .collect();
        let measured: Vec<_> = (0..2000)
            .map(|i| body(&format!("m{i}"), Phase::Measured, &vec![7; 8000]))
            .collect();
        let start = std::time::Instant::now();
        let report = validate_prefix_disjoint(&warm, &measured, 16);
        assert_eq!(report.verdict, Verdict::Fail);
        assert_eq!(report.max_shared_prefix_tokens, 8000);
        assert_eq!(report.max_shared_full_blocks, 500);
        assert_eq!(report.pairs_over_threshold_total, Some(4_000_000));
        assert_eq!(report.pairs_over_threshold.len(), 1000);
        assert!(report.pairs_over_threshold_truncated);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(30),
            "compressed trie must complete this CPU-only study within 30 seconds"
        );
        let trie = TokenTrie::new(&warm);
        assert_eq!(
            trie.nodes.len(),
            2,
            "duplicate long edges must not allocate per token"
        );
    }

    #[test]
    fn compressed_unique_roots_use_one_edge_per_body() {
        let warm: Vec<_> = (0..64u32)
            .map(|index| {
                let mut tokens = vec![7; 8000];
                tokens[0] = index;
                body(&format!("unique-{index}"), Phase::Warmup, &tokens)
            })
            .collect();
        let trie = TokenTrie::new(&warm);
        assert_eq!(trie.nodes.len(), 65);
        for body in &warm {
            let (maximum, range) = trie.query(&warm, &body.token_ids, Some(16));
            assert_eq!(maximum, 8000);
            assert_eq!(range.unwrap().len(), 1);
        }
    }

    #[test]
    fn unreleased_v1_serializes_exact_totals_and_unknown_null() {
        let warm = vec![body("w", Phase::Warmup, &[1, 2])];
        let measured = vec![body("m", Phase::Measured, &[1, 2])];
        let report = validate_prefix_disjoint_with_limits(&warm, &measured, 1, 0, 0);
        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["schema_version"], "kv-token-overlap.v1");
        assert_eq!(value["pairs_over_threshold_total"], 1);
        assert_eq!(value["pairs_over_threshold_truncated"], true);
        assert_eq!(
            serde_json::from_value::<OverlapReport>(value).unwrap(),
            report
        );
        let unknown = validate_prefix_disjoint(&[], &measured, 1);
        let value = serde_json::to_value(&unknown).unwrap();
        assert!(value["pairs_over_threshold_total"].is_null());
        assert_eq!(value["pairs_over_threshold_truncated"], false);
        assert_eq!(
            serde_json::from_value::<OverlapReport>(value).unwrap(),
            unknown
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
