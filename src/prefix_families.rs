// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Deterministic token-first prefix families, checked through the shared validator.

use anyhow::{ensure, Result};
use rand::{rngs::StdRng, seq::SliceRandom, RngExt, SeedableRng};
use serde::{Deserialize, Serialize};

use crate::prefix_validation::{self, OverlapValidation, RenderedRecord, RenderingIdentity};

/// Versioned family construction policy. Counts describe selected requests, not hits.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FamilySpec {
    pub seed: u64,
    pub request_count: usize,
    pub group_count: usize,
    /// Number of shared-prefix-family requests per million requested records.
    pub reuse_fraction_ppm: u32,
    /// Includes common chat root tokens; all lengths refer to final rendered IDs.
    pub prefix_tokens: usize,
    pub suffix_tokens: usize,
    pub output_budget: u64,
    pub common_root: Vec<u32>,
    /// Explicit, ordered, unique token vocabulary; no special-token guessing.
    pub token_pool: Vec<u32>,
    pub popularity: Popularity,
    pub shuffle_order: bool,
    pub max_total_tokens: usize,
}

/// Uniform or finite fixed-point Zipf exponent 1 weights floor(1_000_000/rank).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Popularity {
    Uniform,
    ZipfOne,
}

/// Final checked artifact; planned targets and achieved measurements stay separate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FamilyProfile {
    pub generator_version: String,
    pub spec: FamilySpec,
    pub rows: Vec<RenderedRecord>,
    pub measured: OverlapValidation,
    pub group_counts: std::collections::BTreeMap<String, usize>,
    pub requested_family_requests: usize,
    pub achieved_repeated_requests: usize,
    pub fingerprint: String,
}

/// A renderer must decode and fully render/re-encode the planned IDs with the
/// pinned tokenizer/template. Drift is a failure, never silently relabeled a target.
/// The identity renderer is appropriate only for token-level offline fixtures.
pub fn generate(
    spec: FamilySpec,
    identity: RenderingIdentity,
    block_size: usize,
    mut render: impl FnMut(&[u32]) -> Result<RenderedRecord>,
) -> Result<FamilyProfile> {
    ensure!(
        spec.request_count > 0 && spec.reuse_fraction_ppm <= 1_000_000,
        "invalid family counts or fraction"
    );
    ensure!(
        spec.prefix_tokens >= spec.common_root.len(),
        "prefix shorter than common root"
    );
    let length = spec
        .prefix_tokens
        .checked_add(spec.suffix_tokens)
        .ok_or_else(|| anyhow::anyhow!("length overflow"))?;
    ensure!(
        length > 0
            && length
                .checked_mul(spec.request_count)
                .is_some_and(|n| n <= spec.max_total_tokens),
        "generation token budget exceeded"
    );
    let mut unique = std::collections::BTreeSet::new();
    ensure!(
        !spec.token_pool.is_empty() && spec.token_pool.iter().all(|t| unique.insert(*t)),
        "token pool must be unique and nonempty"
    );
    let family_requests =
        ((spec.request_count as u128 * u128::from(spec.reuse_fraction_ppm)) / 1_000_000) as usize;
    let shared_groups = if family_requests == 0 {
        0
    } else {
        spec.group_count
    };
    ensure!(
        family_requests == 0 || (shared_groups > 0 && shared_groups <= family_requests),
        "not enough requests for declared shared groups"
    );
    let singleton_count = spec.request_count - family_requests;
    let families = shared_groups
        .checked_add(singleton_count)
        .ok_or_else(|| anyhow::anyhow!("family count overflow"))?;
    ensure!(
        families <= spec.token_pool.len(),
        "insufficient vocabulary for prefix-disjoint families"
    );
    ensure!(
        spec.prefix_tokens > spec.common_root.len() || families <= 1,
        "multiple families require a branch token beyond common root"
    );
    let mut rng = StdRng::seed_from_u64(spec.seed);
    let mut branches = spec.token_pool.clone();
    branches.shuffle(&mut rng);
    let mut counts = vec![1usize; families];
    for _ in shared_groups..family_requests {
        let group = match spec.popularity {
            Popularity::Uniform => rng.random_range(0..shared_groups),
            Popularity::ZipfOne => {
                ensure!(
                    shared_groups <= 1_000_000,
                    "Zipf rank exceeds fixed-point precision"
                );
                let weights: Vec<u64> = (1..=shared_groups)
                    .map(|rank| 1_000_000 / rank as u64)
                    .collect();
                let mut draw = rng.random_range(0..weights.iter().sum::<u64>());
                let mut chosen = 0;
                for (i, weight) in weights.iter().enumerate() {
                    if draw < *weight {
                        chosen = i;
                        break;
                    }
                    draw -= weight;
                }
                chosen
            }
        };
        counts[group] += 1;
    }
    let mut rows = Vec::with_capacity(spec.request_count);
    for (group, count) in counts.iter().copied().enumerate() {
        let mut prefix = spec.common_root.clone();
        if prefix.len() < spec.prefix_tokens {
            prefix.push(branches[group]);
        }
        while prefix.len() < spec.prefix_tokens {
            prefix.push(spec.token_pool[rng.random_range(0..spec.token_pool.len())]);
        }
        ensure!(
            spec.suffix_tokens == 0 || count <= spec.token_pool.len(),
            "insufficient unique suffix branch tokens"
        );
        let mut suffix_heads = spec.token_pool.clone();
        suffix_heads.shuffle(&mut rng);
        for repeat in 0..count {
            let mut planned = prefix.clone();
            if spec.suffix_tokens > 0 {
                planned.push(suffix_heads[repeat]);
                for _ in 1..spec.suffix_tokens {
                    planned.push(spec.token_pool[rng.random_range(0..spec.token_pool.len())]);
                }
            }
            let mut row = render(&planned)?;
            ensure!(
                row.tokens == planned,
                "decode/render/re-encode drift from requested family tokens"
            );
            row.record_id = format!("family-{group}-request-{repeat}");
            row.group_id = Some(format!("family-{group}"));
            row.supplied_input_target = Some(length as u64);
            rows.push(row);
        }
    }
    if spec.shuffle_order {
        rows.shuffle(&mut rng);
    }
    let pairs = spec
        .request_count
        .checked_mul(spec.request_count.saturating_sub(1))
        .and_then(|n| n.checked_div(2))
        .ok_or_else(|| anyhow::anyhow!("pair count overflow"))?;
    let measured = prefix_validation::validate(&rows, &[], identity, block_size, pairs, None)?;
    ensure!(
        measured
            .within_group_lcp
            .min
            .is_none_or(|n| n == spec.prefix_tokens),
        "achieved shared prefix mismatch"
    );
    ensure!(
        measured
            .across_group_lcp
            .max
            .is_none_or(|n| n == spec.common_root.len()),
        "inter-group prefix collision"
    );
    let group_counts = counts
        .iter()
        .enumerate()
        .map(|(i, n)| (format!("family-{i}"), *n))
        .collect();
    let achieved_repeated_requests = counts.iter().map(|n| n.saturating_sub(1)).sum();
    let mut profile = FamilyProfile {
        generator_version: "prefix-families.v1".into(),
        spec,
        rows,
        measured,
        group_counts,
        requested_family_requests: family_requests,
        achieved_repeated_requests,
        fingerprint: String::new(),
    };
    profile.fingerprint = crate::public_dataset::sha256(&serde_json::to_vec(&profile)?);
    Ok(profile)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spec() -> FamilySpec {
        FamilySpec {
            seed: 42,
            request_count: 6,
            group_count: 2,
            reuse_fraction_ppm: 1_000_000,
            prefix_tokens: 3,
            suffix_tokens: 2,
            output_budget: 8,
            common_root: vec![999],
            token_pool: (0..32).collect(),
            popularity: Popularity::Uniform,
            shuffle_order: true,
            max_total_tokens: 1000,
        }
    }
    fn identity() -> RenderingIdentity {
        RenderingIdentity {
            tokenizer_id: "fixture/tokens".into(),
            tokenizer_revision: "a".repeat(40),
            tokenizer_sha256: "b".repeat(64),
            chat_template_sha256: "c".repeat(64),
            add_special_tokens: false,
            add_generation_prompt: true,
            hint_nonce_included: false,
            renderer_version: "fixture-v1".into(),
        }
    }
    fn render(tokens: &[u32]) -> Result<RenderedRecord> {
        Ok(RenderedRecord {
            record_id: String::new(),
            group_id: None,
            tokens: tokens.into(),
            rendered_body_sha256: crate::public_dataset::sha256(&serde_json::to_vec(tokens)?),
            supplied_input_target: None,
        })
    }
    #[test]
    fn deterministic_zero_full_mixed_and_zipf() {
        for fraction in [0, 500_000, 1_000_000] {
            for popularity in [Popularity::Uniform, Popularity::ZipfOne] {
                let mut s = spec();
                s.reuse_fraction_ppm = fraction;
                s.popularity = popularity;
                let a = generate(s.clone(), identity(), 2, render).unwrap();
                let b = generate(s, identity(), 2, render).unwrap();
                assert_eq!(a, b);
                assert_eq!(a.rows.len(), 6);
                assert_eq!(a.measured.lengths.min, Some(5));
                if fraction == 0 {
                    assert_eq!(a.achieved_repeated_requests, 0);
                }
                assert_eq!(a.measured.across_group_lcp.max, Some(1));
            }
        }
    }
    #[test]
    fn full_body_repetition_and_zero_prefix() {
        let mut s = spec();
        s.suffix_tokens = 0;
        let a = generate(s, identity(), 2, render).unwrap();
        assert!(a.measured.exact_token_duplicates > 0);
        let mut s = spec();
        s.common_root.clear();
        s.prefix_tokens = 0;
        s.group_count = 1;
        let a = generate(s, identity(), 2, render).unwrap();
        assert_eq!(a.measured.within_group_lcp.max, Some(0));
    }
    #[test]
    fn drift_and_impossible_constraints_fail() {
        assert!(generate(spec(), identity(), 2, |tokens| {
            let mut r = render(tokens)?;
            r.tokens.push(0);
            Ok(r)
        })
        .is_err());
        let mut s = spec();
        s.max_total_tokens = 1;
        assert!(generate(s, identity(), 2, render).is_err());
        let mut s = spec();
        s.token_pool = vec![1, 1];
        assert!(generate(s, identity(), 2, render).is_err());
        let mut s = spec();
        s.reuse_fraction_ppm = 1;
        // floor gives zero shared requests, so the no-reuse singleton control is explicit.
        assert_eq!(
            generate(s, identity(), 2, render)
                .unwrap()
                .requested_family_requests,
            0
        );
        let mut s = spec();
        s.reuse_fraction_ppm = 200_000;
        assert!(generate(s, identity(), 2, render).is_err());
    }
}
