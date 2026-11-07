// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
use metrum_ai_bench::{
    cache_metrics::{probe, ProfileId},
    cache_phase::*,
    cache_rate::*,
};
use std::collections::BTreeMap;
fn snapshot(edge: Edge, rows: &[(&str, u64, u64)]) -> PhaseSnapshot {
    let mut text=String::from("# TYPE vllm:prefix_cache_queries_total counter\n# TYPE vllm:prefix_cache_hits_total counter\n");
    for (worker, q, h) in rows {
        text.push_str(&format!("vllm:prefix_cache_queries_total{{worker=\"{worker}\"}} {q}\nvllm:prefix_cache_hits_total{{worker=\"{worker}\"}} {h}\n"));
    }
    let profile = ProfileId::VllmApc;
    PhaseSnapshot {
        context: BoundaryContext {
            source_key: "PRIVATE_SOURCE".into(),
            run_epoch_key: "PRIVATE_RUN".into(),
            declared_source_revision: Some(profile.source_revision().into()),
            epoch: EpochEvidence::Observed("PRIVATE_EPOCH".into()),
            stage: 1,
            phase: StudyPhase::Warm,
            edge,
            fetch_start_ns: if edge == Edge::Before { 1 } else { 8 },
            fetch_end_ns: if edge == Edge::Before { 2 } else { 9 },
            issuance_or_drain_ns: if edge == Edge::Before { 3 } else { 7 },
        },
        probe: Some(probe(profile, Some(profile.source_revision()), &text).unwrap()),
    }
}
fn labels(names: &[&str]) -> Vec<BTreeMap<String, String>> {
    names
        .iter()
        .map(|name| BTreeMap::from([("worker".into(), (*name).into())]))
        .collect()
}
fn request(workers: &[BTreeMap<String, String>]) -> RateRequest<'_> {
    RateRequest {
        hit_metric: "vllm:prefix_cache_hits_total",
        query_metric: "vllm:prefix_cache_queries_total",
        expected_workers: workers,
        partition: PartitionEvidence::OperatorDeclaredNonOverlapping,
        traffic: TrafficScope::SyntheticExclusive,
        coverage: CoverageMode::Optional,
    }
}
#[test]
fn weighted_phase_deltas_exclude_lifetime_warmup_and_are_not_mean_ratios() {
    let a = snapshot(
        Edge::Before,
        &[("PRIVATE_A", 100, 90), ("PRIVATE_B", 1000, 500)],
    );
    let b = snapshot(
        Edge::After,
        &[("PRIVATE_A", 110, 91), ("PRIVATE_B", 1100, 590)],
    );
    let workers = labels(&["PRIVATE_A", "PRIVATE_B"]);
    let req = request(&workers);
    let result = counter_rate(&a, &b, &req).unwrap();
    assert_eq!(result.hit_delta.as_deref(), Some("91"));
    assert_eq!(result.query_delta.as_deref(), Some("110"));
    assert_eq!(result.ratio, Some(91.0 / 110.0));
    assert_ne!(result.ratio, Some((0.1 + 0.9) / 2.0));
    assert_eq!(result.covered_workers, 2);
    assert!(!serde_json::to_string(&result).unwrap().contains("PRIVATE_"));
}
#[test]
fn weighted_sums_preserve_integer_counts_above_binary64_exact_range() {
    let a = snapshot(Edge::Before, &[("a", 0, 0), ("b", 0, 0)]);
    let b = snapshot(
        Edge::After,
        &[
            ("a", 9_007_199_254_740_991, 9_007_199_254_740_990),
            ("b", 2, 0),
        ],
    );
    let workers = labels(&["a", "b"]);
    let result = counter_rate(&a, &b, &request(&workers)).unwrap();
    assert_eq!(result.query_delta.as_deref(), Some("9007199254740993"));
    assert_eq!(result.hit_delta.as_deref(), Some("9007199254740990"));
    assert!(result.ratio.unwrap() < 1.0);
}
#[test]
fn optional_unknowns_and_required_errors_never_fill_missing_or_reset_series_with_zero() {
    let a = snapshot(Edge::Before, &[("a", 100, 90), ("b", 1000, 500)]);
    let workers = labels(&["a", "b"]);
    for b in [
        snapshot(Edge::After, &[("a", 110, 95)]),
        snapshot(Edge::After, &[("a", 110, 95), ("b", 10, 5)]),
        snapshot(Edge::After, &[("a", 10, 5), ("b", 1100, 590)]),
    ] {
        let mut req = request(&workers);
        let optional = counter_rate(&a, &b, &req).unwrap();
        assert!(optional.ratio.is_none());
        assert!(optional.unknown.is_some());
        assert_eq!(optional.covered_workers, 1);
        assert!(optional.hit_delta.is_none() && optional.query_delta.is_none());
        req.coverage = CoverageMode::Required;
        assert!(counter_rate(&a, &b, &req).is_err());
    }
}
#[test]
fn partition_topology_and_contamination_are_explicit_not_inferred() {
    let a = snapshot(Edge::Before, &[("a", 100, 90), ("b", 1000, 500)]);
    let b = snapshot(Edge::After, &[("a", 110, 91), ("b", 1100, 590)]);
    let workers = labels(&["a", "b"]);
    let mut req = request(&workers);
    req.partition = PartitionEvidence::SingleSeries;
    assert_eq!(
        counter_rate(&a, &b, &req).unwrap().unknown,
        Some(RateUnknown::MissingPartitionDeclaration)
    );
    req.partition = PartitionEvidence::OperatorDeclaredNonOverlapping;
    req.traffic = TrafficScope::SharedOrUnknown;
    assert_eq!(
        counter_rate(&a, &b, &req).unwrap().unknown,
        Some(RateUnknown::SharedTraffic)
    );
    let subset = labels(&["a"]);
    assert_eq!(
        counter_rate(&a, &b, &request(&subset)).unwrap().unknown,
        Some(RateUnknown::UnexpectedPartition)
    );
    let duplicate = labels(&["a", "a"]);
    assert_eq!(
        counter_rate(&a, &b, &request(&duplicate)).unwrap().unknown,
        Some(RateUnknown::DuplicatePartition)
    );
}
#[test]
fn zero_queries_conflicting_counts_and_unrelated_denominators_stay_unknown() {
    let a = snapshot(Edge::Before, &[("a", 100, 90)]);
    let workers = labels(&["a"]);
    let mut req = request(&workers);
    let zero = counter_rate(&a, &snapshot(Edge::After, &[("a", 100, 90)]), &req).unwrap();
    assert_eq!(zero.unknown, Some(RateUnknown::ZeroDenominator));
    assert_eq!(zero.query_delta.as_deref(), Some("0"));
    assert!(zero.ratio.is_none());
    let bad = counter_rate(&a, &snapshot(Edge::After, &[("a", 101, 92)]), &req).unwrap();
    assert_eq!(bad.unknown, Some(RateUnknown::ContradictoryCounters));
    req.query_metric = "vllm:kv_cache_usage_perc";
    assert_eq!(
        counter_rate(&a, &snapshot(Edge::After, &[("a", 110, 91)]), &req)
            .unwrap()
            .unknown,
        Some(RateUnknown::InvalidDefinition)
    );
}
