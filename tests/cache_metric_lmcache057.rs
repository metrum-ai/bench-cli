// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Full upstream MP subscribers on synthetic events; no running engine.
use metrum_ai_bench::cache_metrics::{probe, ProfileId, Role, Unit, Unknown};
use metrum_ai_bench::telemetry::MetricType;
const PAGE: &str = include_str!("fixtures/lmcache057-mp-source-slice.prom");

#[test]
fn mp_source_serialization_preserves_token_chunk_and_event_units() {
    let p = ProfileId::Lmcache057Mp;
    let result = probe(p, Some(p.source_revision()), PAGE).unwrap();
    assert!(result.missing_required_metrics.is_empty());
    for (name, value, unit) in [
        (
            "lmcache_mp_lookup_requested_tokens_total",
            1568.0,
            Unit::Tokens,
        ),
        ("lmcache_mp_lookup_hit_tokens_total", 784.0, Unit::Tokens),
        ("lmcache_mp_l1_write_chunks_total", 2.0, Unit::Chunks),
        ("lmcache_mp_num_chunks_loaded_total", 2.0, Unit::Chunks),
        ("lmcache_mp_num_finished_stores_total", 1.0, Unit::Events),
    ] {
        let row = result
            .observations
            .iter()
            .find(|r| r.spec.metric == name)
            .unwrap();
        assert_eq!(row.value, Some(value));
        assert_eq!(row.spec.kind, MetricType::Counter);
        assert_eq!(row.spec.unit, unit);
    }
    assert!(!p
        .metrics()
        .iter()
        .any(|m| matches!(m.role, Role::Residency | Role::Utilization)));
}

#[test]
fn legacy_and_mp_are_never_source_or_name_aliases() {
    let p = ProfileId::Lmcache057Mp;
    for rev in [
        None,
        Some("236f3eec"),
        Some(ProfileId::Lmcache.source_revision()),
    ] {
        let result = probe(p, rev, PAGE).unwrap();
        assert!(result
            .observations
            .iter()
            .all(|r| r.value.is_none()
                && r.unknown == Some(Unknown::RevisionNotDeclaredOrUnsupported)));
    }
    let legacy = "# TYPE lmcache:num_hit_tokens_total counter\nlmcache:num_hit_tokens_total 64\n";
    let result = probe(p, Some(p.source_revision()), legacy).unwrap();
    assert!(result.observations.is_empty());
    assert_eq!(result.missing_required_metrics.len(), 4);
    let missing = PAGE
        .lines()
        .filter(|line| !line.starts_with("lmcache_mp_lookup_hit_tokens_total{"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        probe(p, Some(p.source_revision()), &missing)
            .unwrap()
            .missing_required_metrics,
        vec!["lmcache_mp_lookup_hit_tokens_total"]
    );
}
