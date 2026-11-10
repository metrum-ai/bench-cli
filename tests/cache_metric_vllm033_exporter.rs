// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Real vLLM logger serialization with injected synthetic scheduler stats.
//! This is not a deployed-engine capture, model execution or GPU evidence.
use metrum_ai_bench::cache_metrics::{probe, ProfileId, Unknown};

const PAGE: &str = include_str!("fixtures/vllm033-logger-synthetic-config.prom");
const OFFLOAD_PAGE: &str = include_str!("fixtures/vllm033-offload-synthetic-stats.prom");

#[test]
fn real_offload_logger_serializes_injected_stats_without_tier_inference() {
    let profile = ProfileId::Vllm4baOffload;
    let result = probe(profile, Some(profile.source_revision()), OFFLOAD_PAGE).unwrap();
    assert!(result.missing_required_metrics.is_empty());
    for (metric, value) in [
        ("vllm:kv_offload_load_bytes_total", 4096.0),
        ("vllm:kv_offload_store_bytes_total", 8192.0),
        ("vllm:kv_offload_load_time_total", 0.01),
        ("vllm:kv_offload_store_time_total", 0.02),
        ("vllm:kv_offload_allocation_failure_total", 1.0),
        ("vllm:kv_offload_lookup_sync_delay_seconds_count", 1.0),
        ("vllm:kv_offload_lookup_sync_delay_seconds_sum", 0.0005),
    ] {
        let row = result
            .observations
            .iter()
            .find(|r| r.spec.metric == metric)
            .unwrap();
        assert_eq!(row.value, Some(value));
        assert_eq!(row.unknown, None);
        assert_eq!(row.labels.get("model_name").unwrap(), "fixture-model");
        assert_eq!(row.labels.get("engine").unwrap(), "0");
    }
    let bucket = result
        .observations
        .iter()
        .find(|r| {
            r.spec.metric == "vllm:kv_offload_lookup_sync_delay_seconds_bucket"
                && r.labels.get("le").is_some_and(|v| v == "0.0005")
        })
        .unwrap();
    assert_eq!(bucket.value, Some(1.0));
    let wrong = probe(
        profile,
        Some(ProfileId::VllmOffload.source_revision()),
        OFFLOAD_PAGE,
    )
    .unwrap();
    assert!(!wrong.observations.is_empty());
    assert!(wrong.observations.iter().all(|row| {
        row.value.is_none() && row.unknown == Some(Unknown::RevisionNotDeclaredOrUnsupported)
    }));
}

#[test]
fn real_logger_exposition_preserves_profile_values_and_unknowns() {
    for (profile, expected) in [
        (
            ProfileId::Vllm4baApc,
            vec![
                ("vllm:prefix_cache_queries_total", 100.0),
                ("vllm:prefix_cache_hits_total", 40.0),
                ("vllm:kv_cache_usage_perc", 0.375),
            ],
        ),
        (
            ProfileId::Vllm4baExternal,
            vec![
                ("vllm:external_prefix_cache_queries_total", 60.0),
                ("vllm:external_prefix_cache_hits_total", 24.0),
            ],
        ),
    ] {
        let result = probe(profile, Some(profile.source_revision()), PAGE).unwrap();
        assert!(result.missing_required_metrics.is_empty());
        for (metric, value) in expected {
            let row = result
                .observations
                .iter()
                .find(|row| row.spec.metric == metric)
                .expect("metric must appear in real logger output");
            assert_eq!(row.value, Some(value));
            assert_eq!(row.unknown, None);
            assert_eq!(row.labels.get("model_name").unwrap(), "fixture-model");
            assert_eq!(row.labels.get("engine").unwrap(), "0");
        }
        let wrong_revision =
            probe(profile, Some(ProfileId::VllmApc.source_revision()), PAGE).unwrap();
        assert!(!wrong_revision.observations.is_empty());
        assert!(wrong_revision.observations.iter().all(|row| {
            row.value.is_none() && row.unknown == Some(Unknown::RevisionNotDeclaredOrUnsupported)
        }));
    }
    // This logger was instantiated without an offloading connector; absence
    // must remain missing rather than turning into a zero or a tier hit.
    let offload = probe(
        ProfileId::Vllm4baOffload,
        Some(ProfileId::Vllm4baOffload.source_revision()),
        PAGE,
    )
    .unwrap();
    assert!(offload.observations.is_empty());
    assert!(!offload.missing_required_metrics.is_empty());
}
