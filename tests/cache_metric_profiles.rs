// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Synthetic source-derived probes, NOT captured live exporter certification.
use metrum_ai_bench::cache_metrics::{probe, ProbeError, ProfileId, Role, Unit, Unknown};
use metrum_ai_bench::telemetry::MetricType;

fn fixture(profile: ProfileId) -> String {
    profile
        .metrics()
        .iter()
        .map(|s| {
            let kind = if s.histogram.is_some() {
                "histogram"
            } else if s.kind == MetricType::Counter {
                "counter"
            } else {
                "gauge"
            };
            let value = if s.unit == Unit::Ratio { "0.5" } else { "10" };
            let tier = if s.metric == "sglang:cached_tokens_total" {
                ",cache_source=\"device\""
            } else {
                ""
            };
            let le = if s.kind == MetricType::HistogramBucket {
                ",le=\"1\""
            } else {
                ""
            };
            format!(
                "# TYPE {} {kind}\n{}{{worker=\"worker-a\"{tier}{le}}} {value}\n",
                s.histogram.map_or(s.metric, |h| h.family),
                s.metric
            )
        })
        .collect()
}

#[test]
fn exact_pinned_profiles_preserve_units_types_labels_and_missing_not_zero() {
    for profile in [
        ProfileId::VllmApc,
        ProfileId::VllmExternal,
        ProfileId::VllmOffload,
        ProfileId::Sglang,
        ProfileId::Lmcache,
        ProfileId::DynamoKvbm,
        ProfileId::TrtLlm,
    ] {
        let page = fixture(profile);
        let result = probe(profile, Some(profile.source_revision()), &page).unwrap();
        assert!(result.missing_required_metrics.is_empty());
        assert_eq!(result.observations.len(), profile.metrics().len());
        for row in result.observations {
            assert!(row.value.is_some());
            assert_eq!(row.labels["worker"], "worker-a");
            if row.spec.role == Role::ReportedHitRate {
                assert_eq!(row.spec.kind, MetricType::Gauge);
            }
        }
        let absent = probe(profile, Some(profile.source_revision()), "").unwrap();
        assert!(absent.observations.is_empty());
        assert!(!absent.missing_required_metrics.is_empty());
        let unknown = probe(profile, None, &page).unwrap();
        assert!(unknown
            .observations
            .iter()
            .all(|v| v.value.is_none()
                && v.unknown == Some(Unknown::RevisionNotDeclaredOrUnsupported)));
    }
    assert!(ProfileId::Lmcache
        .metrics()
        .iter()
        .any(|v| v.role == Role::Stored
            && v.unit == Unit::Tokens
            && v.kind == MetricType::Counter));
    assert!(ProfileId::DynamoKvbm
        .metrics()
        .iter()
        .any(|v| v.unit == Unit::Blocks));
}

#[test]
fn observed_type_not_counter_suffix_and_required_families_gate_values() {
    let p = ProfileId::VllmExternal;
    let page = fixture(p);
    let untyped: String = page
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| format!("{l}\n"))
        .collect();
    assert!(probe(p, Some(p.source_revision()), &untyped)
        .unwrap()
        .observations
        .iter()
        .all(|v| v.unknown == Some(Unknown::TypeNotDeclaredOrConflicting)));
    let incomplete = format!(
        "# TYPE {} counter\n{} 0\n",
        p.metrics()[0].metric,
        p.metrics()[0].metric
    );
    let result = probe(p, Some(p.source_revision()), &incomplete).unwrap();
    assert_eq!(
        result.observations[0].unknown,
        Some(Unknown::MissingRequired)
    );
    let zero = page.replace(" 10\n", " 0\n");
    assert!(probe(p, Some(p.source_revision()), &zero)
        .unwrap()
        .observations
        .iter()
        .all(|v| v.value == Some(0.0)));
    let conflict = format!("{page}# TYPE {} gauge\n", p.metrics()[0].metric);
    assert_eq!(
        probe(p, Some(p.source_revision()), &conflict)
            .unwrap()
            .observations[0]
            .unknown,
        Some(Unknown::TypeNotDeclaredOrConflicting)
    );
}

#[test]
fn duplicate_series_are_unknown_while_distinct_label_sets_are_not_summed() {
    let p = ProfileId::VllmExternal;
    let page = fixture(p);
    let duplicate = format!("{page}{page}");
    assert!(probe(p, Some(p.source_revision()), &duplicate)
        .unwrap()
        .observations
        .iter()
        .all(|v| v.unknown == Some(Unknown::DuplicateIdentity)));
    let extra: String = page
        .replace("worker-a", "worker-b")
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(|line| format!("{line}\n"))
        .collect();
    let distinct = format!("{page}{extra}");
    let result = probe(p, Some(p.source_revision()), &distinct).unwrap();
    assert_eq!(result.observations.len(), 4);
    assert!(result.observations.iter().all(|v| v.value == Some(10.0)));
}

#[test]
fn flat_offload_time_is_continuous_but_bytes_are_integer_and_legacy_totals_are_not_aliases() {
    let p = ProfileId::VllmOffload;
    let page = fixture(p).replace(
        "vllm:kv_offload_load_time_total{worker=\"worker-a\"} 10",
        "vllm:kv_offload_load_time_total{worker=\"worker-a\"} 0.125",
    );
    let page = format!("{page}# TYPE vllm:kv_offload_total_bytes_total counter\nvllm:kv_offload_total_bytes_total{{transfer_type=\"CPU_to_GPU\"}} 999\n");
    let result = probe(p, Some(p.source_revision()), &page).unwrap();
    assert_eq!(result.unmatched_samples, 1);
    let time = result
        .observations
        .iter()
        .find(|row| row.spec.metric == "vllm:kv_offload_load_time_total")
        .unwrap();
    assert_eq!(time.value, Some(0.125));
    assert_eq!(time.spec.unit, Unit::Seconds);
    let bad = page.replace(
        "vllm:kv_offload_load_bytes_total{worker=\"worker-a\"} 10",
        "vllm:kv_offload_load_bytes_total{worker=\"worker-a\"} 0.125",
    );
    assert_eq!(
        probe(p, Some(p.source_revision()), &bad)
            .unwrap()
            .observations
            .iter()
            .find(|row| row.spec.metric == "vllm:kv_offload_load_bytes_total")
            .unwrap()
            .unknown,
        Some(Unknown::InvalidValue)
    );
    let specs = ProfileId::Lmcache.metrics();
    assert!(specs.iter().any(|s| s.role == Role::Residency
        && s.kind == MetricType::Gauge
        && s.unit == Unit::Bytes));
    assert!(specs
        .iter()
        .any(|s| s.role == Role::Eviction && s.unit == Unit::Keys));
    assert_eq!(
        specs
            .iter()
            .find(|s| s.metric == "lmcache:local_cpu_evict_failed_count_total")
            .unwrap()
            .unit,
        Unit::EvictionCandidates
    );
}

#[test]
fn histogram_parent_type_and_unit_are_not_inferred_from_scalar_suffixes() {
    let p = ProfileId::Lmcache;
    let family = "lmcache:time_to_retrieve";
    let base = fixture(p).replace(
        "lmcache:time_to_retrieve_sum{worker=\"worker-a\"} 10",
        "lmcache:time_to_retrieve_sum{worker=\"worker-a\"} 0.125",
    );
    let page = format!("# UNIT {family} seconds\n{base}");
    let result = probe(p, Some(p.source_revision()), &page).unwrap();
    let hist: Vec<_> = result
        .observations
        .iter()
        .filter(|r| r.spec.histogram.is_some())
        .collect();
    assert_eq!(hist.len(), 3);
    assert!(hist.iter().all(|r| r.value.is_some()));
    assert_eq!(
        hist.iter()
            .find(|r| r.spec.metric.ends_with("_sum"))
            .unwrap()
            .value,
        Some(0.125)
    );
    assert!(hist
        .iter()
        .filter(|r| !r.spec.metric.ends_with("_sum"))
        .all(|r| r.spec.unit == Unit::Observations));
    for changed in [
        page.replace(
            "# UNIT lmcache:time_to_retrieve seconds",
            "# UNIT lmcache:time_to_retrieve bytes",
        ),
        page.replace(
            "# TYPE lmcache:time_to_retrieve histogram",
            "# TYPE lmcache:time_to_retrieve counter",
        ),
        format!(
            "{}# TYPE {family} histogram\n",
            page.replace("# TYPE lmcache:time_to_retrieve histogram\n", "")
        ),
    ] {
        assert!(probe(p, Some(p.source_revision()), &changed)
            .unwrap()
            .observations
            .iter()
            .filter(|r| r.spec.histogram.is_some())
            .all(|r| r.value.is_none()));
    }
    let invalid = page.replace("le=\"1\"", "le=\"NaN\"");
    assert_eq!(
        probe(p, Some(p.source_revision()), &invalid)
            .unwrap()
            .observations
            .iter()
            .find(|r| r.spec.kind == MetricType::HistogramBucket)
            .unwrap()
            .unknown,
        Some(Unknown::InvalidLabels)
    );
}

#[test]
fn explicit_incompatible_units_cannot_override_source_semantics() {
    let p = ProfileId::VllmExternal;
    let page = format!("# UNIT {} bytes\n{}", p.metrics()[0].metric, fixture(p));
    let result = probe(p, Some(p.source_revision()), &page).unwrap();
    assert_eq!(
        result.observations[0].unknown,
        Some(Unknown::UnitNotCompatible)
    );
    assert!(result.observations[0].value.is_none());
    assert_eq!(result.observations[1].value, Some(10.0));
}

#[test]
fn unrelated_series_do_not_exhaust_the_selected_profile_budget() {
    let p = ProfileId::VllmExternal;
    let unrelated: String = (0..1024)
        .map(|i| format!("other_metric{{index=\"{i}\"}} 1\n"))
        .collect();
    let result = probe(
        p,
        Some(p.source_revision()),
        &format!("{}{unrelated}", fixture(p)),
    )
    .unwrap();
    assert_eq!(result.observations.len(), 2);
    assert_eq!(result.unmatched_samples, 1024);
    assert!(result.observations.iter().all(|v| v.value == Some(10.0)));
    let too_many: String = (0..257)
        .map(|i| format!("{}{{worker=\"{i}\"}} 1\n", p.metrics()[0].metric))
        .collect();
    assert_eq!(
        probe(p, Some(p.source_revision()), &too_many).unwrap_err(),
        ProbeError::TooManySeries
    );
}

#[test]
fn late_types_and_unknown_storage_sources_are_not_validated() {
    let p = ProfileId::VllmExternal;
    let page = fixture(p);
    let samples: String = page
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| format!("{l}\n"))
        .collect();
    let declarations: String = page
        .lines()
        .filter(|l| l.starts_with('#'))
        .map(|l| format!("{l}\n"))
        .collect();
    let result = probe(
        p,
        Some(p.source_revision()),
        &format!("{samples}{declarations}"),
    )
    .unwrap();
    assert_eq!(result.observations.len(), 2);
    assert!(result
        .observations
        .iter()
        .all(|v| v.unknown == Some(Unknown::TypeNotDeclaredOrConflicting)));
    let p = ProfileId::Sglang;
    let page = fixture(p).replace(
        "cache_source=\"device\"",
        "cache_source=\"unvalidated-tier\"",
    );
    let result = probe(p, Some(p.source_revision()), &page).unwrap();
    assert_eq!(result.observations[0].unknown, Some(Unknown::InvalidLabels));
}

#[test]
fn invalid_nonfinite_fractional_and_imprecise_counters_remain_unknown() {
    let p = ProfileId::VllmExternal;
    for value in ["NaN", "+Inf", "-1", "0.5", "9007199254740992"] {
        let page = fixture(p).replace(" 10\n", &format!(" {value}\n"));
        assert!(probe(p, Some(p.source_revision()), &page)
            .unwrap()
            .observations
            .iter()
            .all(|v| v.value.is_none()));
    }
    let p = ProfileId::VllmApc;
    let result = probe(
        p,
        Some(p.source_revision()),
        &fixture(p).replace(" 0.5\n", " 50\n"),
    )
    .unwrap();
    assert_eq!(
        result.observations.last().unwrap().unknown,
        Some(Unknown::InvalidValue)
    );
    assert_eq!(
        probe(
            p,
            Some(p.source_revision()),
            &"x".repeat(4 * 1024 * 1024 + 1)
        )
        .unwrap_err(),
        ProbeError::PageTooLarge
    );
}
