// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Capacity-aware logical pressure plans. No physical cache observations.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Per-rank classic K/V geometry, explicitly accounting for replicated KV heads.
/// These are effective per-rank values, never blindly divided by tensor parallelism.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct KvRank {
    pub usable_cache_bytes: u64,
    pub effective_layers: u64,
    pub kv_heads_per_rank: u64,
    pub head_dimension: u64,
    pub dtype_bytes: u64,
}

/// Configured bottleneck capacity, not a measurement of allocated/resident KV.
/// Empty geometry retains unknown. Non-classic layouts must supply an explicit
/// capacity elsewhere rather than pretending to follow this formula.
pub fn classic_capacity_blocks(ranks: &[KvRank], block_tokens: u64) -> Result<Option<u64>> {
    ensure!(block_tokens > 0, "positive block size required");
    let mut capacity = None;
    for rank in ranks {
        let factors = [
            2,
            rank.effective_layers,
            rank.kv_heads_per_rank,
            rank.head_dimension,
            rank.dtype_bytes,
            block_tokens,
        ];
        let mut bytes = 1u64;
        for factor in factors {
            ensure!(factor > 0, "positive effective KV geometry required");
            bytes = bytes
                .checked_mul(factor)
                .context("KV block geometry overflow")?;
        }
        let blocks = rank.usable_cache_bytes / bytes;
        capacity = Some(capacity.map_or(blocks, |old: u64| old.min(blocks)));
    }
    Ok(capacity)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PressureBand {
    BelowCapacity,
    AboveCapacity,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PressurePhase {
    Populate,
    Pressure,
    Rehit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogicalRequest {
    pub phase: PressurePhase,
    /// Globally interned prefix-tree node IDs, NOT individual source hash labels.
    pub prefix_nodes: Vec<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PressureConfig {
    pub capacity_blocks: Option<u64>,
    pub hot_blocks: u64,
    pub pressure_blocks: u64,
    pub band: PressureBand,
    pub max_plan_nodes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PressurePlan {
    pub schema_version: String,
    pub configured_capacity_blocks: u64,
    pub distinct_full_nodes: u64,
    pub band: PressureBand,
    pub requests: Vec<LogicalRequest>,
    pub rendered_token_overlap: String,
    pub physical_cache_evidence: String,
}

/// Produce populate-A -> distinct-pressure-B -> re-hit-A. Capacity is required
/// to label a band. All nodes are modeled complete/cacheable blocks; uncertain
/// tails cannot establish an above-capacity claim. No text/tokens are fabricated.
pub fn plan(config: PressureConfig) -> Result<PressurePlan> {
    let capacity = config
        .capacity_blocks
        .context("capacity unknown: cannot label pressure band")?;
    ensure!(
        capacity > 0 && config.hot_blocks > 0 && config.pressure_blocks > 0,
        "positive capacity/hot/pressure blocks required"
    );
    ensure!(
        config.hot_blocks <= capacity,
        "populate working set must fit configured capacity"
    );
    let total = config
        .hot_blocks
        .checked_add(config.pressure_blocks)
        .context("pressure node overflow")?;
    ensure!(
        total <= config.max_plan_nodes && usize::try_from(total).is_ok(),
        "pressure plan node limit exceeded"
    );
    match config.band {
        PressureBand::BelowCapacity => ensure!(
            total < capacity,
            "below-capacity plan must be strictly below capacity"
        ),
        PressureBand::AboveCapacity => {
            ensure!(total > capacity, "above-capacity plan must exceed capacity")
        }
    }
    let hot: Vec<_> = (0..config.hot_blocks).collect();
    let pressure = (config.hot_blocks..total).collect();
    Ok(PressurePlan {
        schema_version: "logical-pressure-plan.v1".into(),
        configured_capacity_blocks: capacity,
        distinct_full_nodes: total,
        band: config.band,
        requests: vec![
            LogicalRequest {
                phase: PressurePhase::Populate,
                prefix_nodes: hot.clone(),
            },
            LogicalRequest {
                phase: PressurePhase::Pressure,
                prefix_nodes: pressure,
            },
            LogicalRequest {
                phase: PressurePhase::Rehit,
                prefix_nodes: hot,
            },
        ],
        rendered_token_overlap: "unknown".into(),
        physical_cache_evidence: "unavailable".into(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModeledAccess {
    pub phase: PressurePhase,
    pub resident_hits: u64,
    pub misses: u64,
    pub modeled_evictions: u64,
}

/// A deterministic CPU-only LRU oracle, NOT actual serving/cache-tier telemetry.
/// Unknown capacity returns None, never a passing residence/hit assertion.
pub fn simulate_lru(
    requests: &[LogicalRequest],
    capacity: Option<u64>,
    max_nodes: usize,
) -> Result<Option<Vec<ModeledAccess>>> {
    let Some(capacity) = capacity else {
        return Ok(None);
    };
    ensure!(
        capacity > 0 && capacity <= max_nodes as u64,
        "simulation capacity outside limits"
    );
    let mut parents = BTreeMap::new();
    let mut resident = BTreeMap::new();
    let mut clock = 0u64;
    let mut result = Vec::new();
    for request in requests {
        let mut parent = None;
        let mut access = ModeledAccess {
            phase: request.phase,
            resident_hits: 0,
            misses: 0,
            modeled_evictions: 0,
        };
        for node in &request.prefix_nodes {
            if let Some(old) = parents.get(node) {
                ensure!(*old == parent, "conflicting prefix-tree node parent");
            } else {
                ensure!(
                    parents.len() < max_nodes,
                    "simulation distinct-node limit exceeded"
                );
                parents.insert(*node, parent);
            }
            parent = Some(*node);
            clock = clock.checked_add(1).context("simulation clock overflow")?;
            if resident.contains_key(node) {
                access.resident_hits += 1;
            } else {
                access.misses += 1;
                if resident.len() as u64 == capacity {
                    let victim = *resident
                        .iter()
                        .min_by_key(|(id, age)| (**age, **id))
                        .context("empty modeled cache")?
                        .0;
                    resident.remove(&victim);
                    access.modeled_evictions += 1;
                }
            }
            resident.insert(*node, clock);
        }
        result.push(access);
    }
    Ok(Some(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn effective_rank_geometry_respects_replication_and_bottleneck() {
        let rank = KvRank {
            usable_cache_bytes: 1024,
            effective_layers: 2,
            kv_heads_per_rank: 2,
            head_dimension: 4,
            dtype_bytes: 2,
        };
        assert_eq!(classic_capacity_blocks(&[rank], 4).unwrap(), Some(4));
        assert_eq!(
            classic_capacity_blocks(
                &[
                    rank,
                    KvRank {
                        usable_cache_bytes: 512,
                        ..rank
                    }
                ],
                4
            )
            .unwrap(),
            Some(2)
        );
        assert_eq!(classic_capacity_blocks(&[], 16).unwrap(), None);
        assert!(classic_capacity_blocks(
            &[KvRank {
                effective_layers: u64::MAX,
                ..rank
            }],
            16
        )
        .is_err());
        assert!(classic_capacity_blocks(
            &[KvRank {
                kv_heads_per_rank: 0,
                ..rank
            }],
            16
        )
        .is_err());
    }
    #[test]
    fn below_and_above_capacity_rehit_have_distinct_modeled_outcomes() {
        let config = PressureConfig {
            capacity_blocks: Some(5),
            hot_blocks: 2,
            pressure_blocks: 2,
            band: PressureBand::BelowCapacity,
            max_plan_nodes: 100,
        };
        let below = plan(config).unwrap();
        assert_eq!(below.rendered_token_overlap, "unknown");
        assert_eq!(below.physical_cache_evidence, "unavailable");
        let hits = simulate_lru(&below.requests, Some(5), 100)
            .unwrap()
            .unwrap();
        assert_eq!(hits[2].resident_hits, 2);
        let above = plan(PressureConfig {
            pressure_blocks: 6,
            band: PressureBand::AboveCapacity,
            ..config
        })
        .unwrap();
        let misses = simulate_lru(&above.requests, Some(5), 100)
            .unwrap()
            .unwrap();
        assert_eq!(misses[2].misses, 2);
        assert_eq!(simulate_lru(&above.requests, None, 100).unwrap(), None);
        assert!(plan(PressureConfig {
            capacity_blocks: None,
            ..config
        })
        .is_err());
        assert!(plan(PressureConfig {
            pressure_blocks: 3,
            ..config
        })
        .is_err());
        assert!(plan(PressureConfig {
            max_plan_nodes: 1,
            ..config
        })
        .is_err());
    }
    #[test]
    fn conflicting_node_paths_and_limits_fail_not_merge_labels() {
        let requests = vec![
            LogicalRequest {
                phase: PressurePhase::Populate,
                prefix_nodes: vec![1, 2],
            },
            LogicalRequest {
                phase: PressurePhase::Rehit,
                prefix_nodes: vec![3, 2],
            },
        ];
        assert!(simulate_lru(&requests, Some(2), 10).is_err());
        assert!(simulate_lru(&requests, Some(2), 1).is_err());
    }
}
