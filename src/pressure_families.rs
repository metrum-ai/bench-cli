// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Reproducible logical cyclic/integer-Zipf source schedules, not live traffic.
use crate::pressure_inputs::{LogicalRequest, PressurePhase};
use crate::public_dataset::sha256;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Family {
    Cyclic,
    IntegerZipfOne { weight_scale: u64 },
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FamilyConfig {
    pub family: Family,
    pub groups: u32,
    pub blocks_per_group: u32,
    pub requests: u64,
    pub seed: u64,
    pub configured_capacity_blocks: Option<u64>,
    pub max_groups: u32,
    pub max_requests: u64,
    pub max_unique_nodes: u64,
    pub max_scheduled_nodes: u64,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapacityRelation {
    Unknown,
    Below,
    Equal,
    Above,
}
fn relation(footprint: u64, capacity: Option<u64>) -> CapacityRelation {
    match capacity {
        None => CapacityRelation::Unknown,
        Some(n) if footprint < n => CapacityRelation::Below,
        Some(n) if footprint == n => CapacityRelation::Equal,
        Some(_) => CapacityRelation::Above,
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FamilyPlan {
    pub schema_version: String,
    pub config: FamilyConfig,
    pub requested_pool_nodes: u64,
    pub achieved_unique_nodes: u64,
    pub requested_pool_relation: CapacityRelation,
    pub achieved_footprint_relation: CapacityRelation,
    pub achieved_group_counts: Vec<u64>,
    pub effective_group_order: Vec<u32>,
    pub requests: Vec<LogicalRequest>,
    pub rendered_token_overlap: String,
    pub physical_cache_evidence: String,
    pub plan_sha256: String,
}

/// Rejection sampling avoids modulo bias. Bounded SHA-based draws are a
/// deterministic construction, not a claim to reproduce production arrivals.
fn draw(seed: u64, index: u64, range: u64) -> Result<u64> {
    ensure!(range > 0, "positive sampler range required");
    let threshold = range.wrapping_neg() % range;
    for attempt in 0..64u8 {
        let digest = sha256(&serde_json::to_vec(&(seed, index, attempt))?);
        let value = u64::from_str_radix(&digest[..16], 16)?;
        if value >= threshold {
            return Ok(value % range);
        }
    }
    anyhow::bail!("deterministic sampler attempt limit")
}

/// Build independent logical group chains. Each group/node identity is
/// constructed; no input lengths, source hashes or text are recovered. Integer
/// Zipf uses floor(weight_scale/rank) at exponent exactly one, not arbitrary Zipf.
/// Report achieved counts/order/footprint; requested pool size is not achieved load.
pub fn generate(config: FamilyConfig) -> Result<FamilyPlan> {
    ensure!(
        config.groups > 0 && config.groups <= config.max_groups && config.blocks_per_group > 0,
        "positive bounded groups/block count required"
    );
    ensure!(
        config.requests > 0
            && config.requests <= config.max_requests
            && usize::try_from(config.requests).is_ok(),
        "request limit"
    );
    ensure!(
        config.configured_capacity_blocks.is_none_or(|n| n > 0),
        "known capacity must be positive"
    );
    let pool = u64::from(config.groups)
        .checked_mul(u64::from(config.blocks_per_group))
        .context("pool node overflow")?;
    let scheduled = config
        .requests
        .checked_mul(u64::from(config.blocks_per_group))
        .context("scheduled node overflow")?;
    ensure!(
        pool <= config.max_unique_nodes && scheduled <= config.max_scheduled_nodes,
        "logical node limits exceeded"
    );
    let mut cumulative = Vec::new();
    let mut total = 0u64;
    if let Family::IntegerZipfOne { weight_scale } = config.family {
        ensure!(
            weight_scale >= u64::from(config.groups),
            "Zipf scale must retain positive support for every group"
        );
        for rank in 1..=config.groups {
            total = total
                .checked_add(weight_scale / u64::from(rank))
                .context("Zipf weight sum overflow")?;
            cumulative.push(total);
        }
    }
    let mut counts = vec![0u64; config.groups as usize];
    let mut order = Vec::new();
    let mut requests = Vec::new();
    for index in 0..config.requests {
        let group = match config.family {
            Family::Cyclic => {
                ((index % u64::from(config.groups) + config.seed % u64::from(config.groups))
                    % u64::from(config.groups)) as usize
            }
            Family::IntegerZipfOne { .. } => {
                let chosen = draw(config.seed, index, total)?;
                cumulative.partition_point(|weight| *weight <= chosen)
            }
        };
        let phase = if counts[group] == 0 {
            PressurePhase::Populate
        } else {
            PressurePhase::Rehit
        };
        counts[group] = counts[group]
            .checked_add(1)
            .context("group count overflow")?;
        let start = group as u64 * u64::from(config.blocks_per_group);
        requests.push(LogicalRequest {
            phase,
            prefix_nodes: (start..start + u64::from(config.blocks_per_group)).collect(),
        });
        order.push(group as u32);
    }
    let achieved =
        counts.iter().filter(|n| **n > 0).count() as u64 * u64::from(config.blocks_per_group);
    let digest = sha256(&serde_json::to_vec(&(
        "logical-pressure-family.v1",
        config,
        &order,
        &counts,
        &requests,
    ))?);
    Ok(FamilyPlan {
        schema_version: "logical-pressure-family.v1".into(),
        config,
        requested_pool_nodes: pool,
        achieved_unique_nodes: achieved,
        requested_pool_relation: relation(pool, config.configured_capacity_blocks),
        achieved_footprint_relation: relation(achieved, config.configured_capacity_blocks),
        achieved_group_counts: counts,
        effective_group_order: order,
        requests,
        rendered_token_overlap: "unknown".into(),
        physical_cache_evidence: "unavailable".into(),
        plan_sha256: digest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> FamilyConfig {
        FamilyConfig {
            family: Family::Cyclic,
            groups: 4,
            blocks_per_group: 2,
            requests: 12,
            seed: 0,
            configured_capacity_blocks: Some(4),
            max_groups: 100,
            max_requests: 10000,
            max_unique_nodes: 1000,
            max_scheduled_nodes: 100000,
        }
    }
    #[test]
    fn cyclic_rehits_and_achieved_capacity_are_exact() {
        let plan = generate(config()).unwrap();
        assert_eq!(
            plan.effective_group_order,
            vec![0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3]
        );
        assert_eq!(plan.achieved_group_counts, vec![3, 3, 3, 3]);
        assert_eq!(plan.requests[0].phase, PressurePhase::Populate);
        assert_eq!(plan.requests[4].phase, PressurePhase::Rehit);
        assert_eq!(plan.achieved_footprint_relation, CapacityRelation::Above);
        let short = generate(FamilyConfig {
            requests: 1,
            ..config()
        })
        .unwrap();
        assert_eq!(short.requested_pool_relation, CapacityRelation::Above);
        assert_eq!(short.achieved_footprint_relation, CapacityRelation::Below);
        assert_eq!(short.achieved_unique_nodes, 2);
        assert_eq!(
            generate(FamilyConfig {
                configured_capacity_blocks: None,
                ..config()
            })
            .unwrap()
            .achieved_footprint_relation,
            CapacityRelation::Unknown
        );
    }
    #[test]
    fn integer_zipf_seed_order_and_requested_vs_achieved_are_reported() {
        let c = FamilyConfig {
            family: Family::IntegerZipfOne { weight_scale: 120 },
            requests: 1000,
            ..config()
        };
        let a = generate(c).unwrap();
        let b = generate(c).unwrap();
        assert_eq!(a.plan_sha256, b.plan_sha256);
        assert_eq!(a.effective_group_order, b.effective_group_order);
        assert_eq!(a.achieved_group_counts.iter().sum::<u64>(), 1000);
        // Independent Python SHA-256/bisect implementation produced this golden;
        // exact deterministic counts/order, not a flaky statistical tolerance.
        assert_eq!(a.achieved_group_counts, vec![493, 219, 157, 131]);
        assert_eq!(
            &a.effective_group_order[..16],
            &[3, 2, 1, 3, 0, 0, 0, 2, 0, 1, 0, 1, 1, 0, 1, 1]
        );
        assert_ne!(
            a.plan_sha256,
            generate(FamilyConfig { seed: 1, ..c }).unwrap().plan_sha256
        );
        assert_eq!(a.rendered_token_overlap, "unknown");
        assert_eq!(a.physical_cache_evidence, "unavailable");
    }
    #[test]
    fn schema_roundtrip_body_fingerprint_and_negative_evidence() {
        let plan = generate(config()).unwrap();
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../schemas/logical-pressure-family.v1.json"))
                .unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        let value = serde_json::to_value(&plan).unwrap();
        assert!(validator.is_valid(&value));
        let restored: FamilyPlan = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(restored.requests, plan.requests);
        let fingerprint = |requests: &[LogicalRequest]| {
            sha256(
                &serde_json::to_vec(&(
                    "logical-pressure-family.v1",
                    plan.config,
                    &plan.effective_group_order,
                    &plan.achieved_group_counts,
                    requests,
                ))
                .unwrap(),
            )
        };
        assert_eq!(plan.plan_sha256, fingerprint(&plan.requests));
        let mut changed = plan.requests.clone();
        changed[0].prefix_nodes[0] = 999;
        assert_ne!(plan.plan_sha256, fingerprint(&changed));
        let mut false_evidence = value;
        false_evidence["physical_cache_evidence"] = serde_json::json!("observed");
        assert!(!validator.is_valid(&false_evidence));
        assert_eq!(
            generate(FamilyConfig {
                configured_capacity_blocks: Some(8),
                ..config()
            })
            .unwrap()
            .achieved_footprint_relation,
            CapacityRelation::Equal
        );
        assert_eq!(
            generate(FamilyConfig {
                seed: u64::MAX,
                ..config()
            })
            .unwrap()
            .effective_group_order[0],
            3
        );
    }

    #[test]
    fn malformed_limits_support_and_overflow_fail() {
        assert!(generate(FamilyConfig {
            groups: 0,
            ..config()
        })
        .is_err());
        assert!(generate(FamilyConfig {
            max_scheduled_nodes: 1,
            ..config()
        })
        .is_err());
        assert!(generate(FamilyConfig {
            family: Family::IntegerZipfOne { weight_scale: 1 },
            ..config()
        })
        .is_err());
        assert!(generate(FamilyConfig {
            family: Family::IntegerZipfOne {
                weight_scale: u64::MAX
            },
            ..config()
        })
        .is_err());
        assert!(generate(FamilyConfig {
            configured_capacity_blocks: Some(0),
            ..config()
        })
        .is_err());
    }
}
