// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! CPU-only exclusive two-tier LRU oracle; never physical cache observations.
use crate::pressure_inputs::{simulate_lru, LogicalRequest, PressurePhase};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModeledTierAccess {
    pub phase: PressurePhase,
    pub resident_hits: u64,
    pub external_hits: u64,
    pub misses: u64,
    pub primary_evictions: u64,
    pub secondary_evictions: u64,
}
fn oldest(cache: &BTreeMap<u64, u64>) -> Result<u64> {
    cache
        .iter()
        .min_by_key(|(id, age)| (**age, **id))
        .map(|(id, _)| *id)
        .context("empty modeled tier")
}

/// Exclusive LRU: primary eviction demotes to secondary; external hit promotes.
/// All counts are logical node lookups, not saved prefill or contiguous-prefix
/// hits. Unknown capacity returns None, never a passing tier-participation claim.
pub fn simulate(
    requests: &[LogicalRequest],
    primary: Option<u64>,
    secondary: Option<u64>,
    max_nodes: usize,
) -> Result<Option<Vec<ModeledTierAccess>>> {
    let (Some(primary), Some(secondary)) = (primary, secondary) else {
        return Ok(None);
    };
    let total = primary
        .checked_add(secondary)
        .context("modeled tier capacity overflow")?;
    ensure!(
        primary > 0 && total <= max_nodes as u64,
        "invalid modeled tier capacity"
    );
    // Reuse the single-tier oracle's bounded path-consistency validation.
    simulate_lru(requests, Some(total), max_nodes)?;
    let mut l1 = BTreeMap::new();
    let mut l2 = BTreeMap::new();
    let mut clock = 0u64;
    let mut output = Vec::new();
    for request in requests {
        let mut stats = ModeledTierAccess {
            phase: request.phase,
            resident_hits: 0,
            external_hits: 0,
            misses: 0,
            primary_evictions: 0,
            secondary_evictions: 0,
        };
        for node in &request.prefix_nodes {
            clock = clock
                .checked_add(1)
                .context("modeled tier clock overflow")?;
            if l1.contains_key(node) {
                stats.resident_hits += 1;
                l1.insert(*node, clock);
                continue;
            }
            if l2.remove(node).is_some() {
                stats.external_hits += 1;
            } else {
                stats.misses += 1;
            }
            if l1.len() as u64 == primary {
                let victim = oldest(&l1)?;
                l1.remove(&victim);
                stats.primary_evictions += 1;
                if secondary > 0 {
                    if l2.len() as u64 == secondary {
                        l2.remove(&oldest(&l2)?);
                        stats.secondary_evictions += 1;
                    }
                    l2.insert(victim, clock);
                }
            }
            l1.insert(*node, clock);
        }
        output.push(stats);
    }
    Ok(Some(output))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(phase: PressurePhase, id: u64) -> LogicalRequest {
        LogicalRequest {
            phase,
            prefix_nodes: vec![id],
        }
    }
    #[test]
    fn resident_external_and_recompute_cases_are_modeled_not_observed() {
        let requests = vec![
            request(PressurePhase::Populate, 1),
            request(PressurePhase::Rehit, 1),
            request(PressurePhase::Pressure, 2),
            request(PressurePhase::Rehit, 1),
        ];
        let result = simulate(&requests, Some(1), Some(1), 10).unwrap().unwrap();
        assert_eq!(result[0].misses, 1);
        assert_eq!(result[1].resident_hits, 1);
        assert_eq!(result[3].external_hits, 1);
        let no_external = simulate(&requests, Some(1), Some(0), 10).unwrap().unwrap();
        assert_eq!(no_external[3].misses, 1);
        assert_eq!(no_external[3].external_hits, 0);
    }
    #[test]
    fn secondary_eviction_and_unknown_capacity_never_claim_hits() {
        let requests = vec![
            request(PressurePhase::Populate, 1),
            request(PressurePhase::Pressure, 2),
            request(PressurePhase::Pressure, 3),
            request(PressurePhase::Rehit, 1),
        ];
        let result = simulate(&requests, Some(1), Some(1), 10).unwrap().unwrap();
        assert_eq!(result[2].secondary_evictions, 1);
        assert_eq!(result[3].misses, 1);
        assert_eq!(simulate(&requests, None, Some(1), 10).unwrap(), None);
        assert!(simulate(&requests, Some(u64::MAX), Some(1), 10).is_err());
        assert!(simulate(&requests, Some(0), Some(1), 10).is_err());
    }
}
