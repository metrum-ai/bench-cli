// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Upstream metric statements with synthetic inputs; no deployed engine or GPU.
use metrum_ai_bench::cache_metric_capture::ProfileCapabilities;
use metrum_ai_bench::cache_metrics::{probe, ProfileId, Role, Unit, Unknown};
use metrum_ai_bench::telemetry::MetricType;
const PAGE: &str = include_str!("fixtures/sglang0523-source-slice.prom");

#[test]
fn source_slice_preserves_tiers_and_gauge_without_occupancy_inference() {
    let profile = ProfileId::Sglang0523;
    let result = probe(profile, Some(profile.source_revision()), PAGE).unwrap();
    assert!(result.missing_required_metrics.is_empty());
    assert_eq!(result.observations.len(), 4);
    for (source, value) in [("device", 32.0), ("host", 48.0), ("total", 16.0)] {
        let row = result
            .observations
            .iter()
            .find(|r| r.labels.get("cache_source").is_some_and(|s| s == source))
            .unwrap();
        assert_eq!(row.value, Some(value));
        assert_eq!(row.spec.unit, Unit::Tokens);
        assert_eq!(row.labels["tp_rank"], "0");
    }
    let rate = result
        .observations
        .iter()
        .find(|r| r.spec.role == Role::ReportedHitRate)
        .unwrap();
    assert_eq!(rate.value, Some(0.625));
    assert_eq!(rate.spec.kind, MetricType::Gauge);
    assert!(!profile
        .metrics()
        .iter()
        .any(|m| matches!(m.role, Role::Residency | Role::Utilization)));
    assert!(
        !ProfileCapabilities::new(profile, Some(profile.source_revision()))
            .physical_tier_validation
    );
    let only_rate = probe(
        profile,
        Some(profile.source_revision()),
        "# TYPE sglang:cache_hit_rate gauge\nsglang:cache_hit_rate 0.75\n",
    )
    .unwrap();
    assert_eq!(
        only_rate.missing_required_metrics,
        vec!["sglang:cached_tokens_total"]
    );
}

#[test]
fn revisions_and_invalid_rate_remain_unknown() {
    let profile = ProfileId::Sglang0523;
    for revision in [
        None,
        Some("297e54fd"),
        Some(ProfileId::Sglang.source_revision()),
    ] {
        let result = probe(profile, revision, PAGE).unwrap();
        assert!(result
            .observations
            .iter()
            .all(|r| r.value.is_none()
                && r.unknown == Some(Unknown::RevisionNotDeclaredOrUnsupported)));
    }
    let old = probe(ProfileId::Sglang, Some(profile.source_revision()), PAGE).unwrap();
    assert!(old.observations.iter().all(|r| r.value.is_none()));
    for value in ["1.1", "NaN", "-0.1"] {
        let page = PAGE.replace(" 0.625", &format!(" {value}"));
        let result = probe(profile, Some(profile.source_revision()), &page).unwrap();
        assert!(result
            .observations
            .iter()
            .find(|r| r.spec.role == Role::ReportedHitRate)
            .unwrap()
            .value
            .is_none());
    }
}
