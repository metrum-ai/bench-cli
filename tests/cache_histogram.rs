// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
//! Source-derived synthetic components, not live exporter or quantile proof.
use metrum_ai_bench::{
    cache_histogram::*,
    cache_metrics::{probe, ProfileId, Unit},
    cache_phase::*,
    telemetry::MetricType,
};
use std::collections::{BTreeMap, BTreeSet};
const FAMILY: &str = "lmcache:time_to_retrieve";
fn page(after: bool) -> String {
    let profile = ProfileId::Lmcache;
    let mut text = String::new();
    let mut families = BTreeSet::new();
    for spec in profile.metrics() {
        if let Some(hist) = spec.histogram {
            if !families.insert(hist.family) {
                continue;
            }
            let unit = serde_json::to_value(hist.unit).unwrap();
            let unit = unit.as_str().unwrap();
            let (low, count, sum) = if after { (8, 14, 28) } else { (5, 10, 20) };
            text.push_str(&format!("# TYPE {} histogram\n# UNIT {} {}\n{}_bucket{{worker=\"PRIVATE_WORKER\",le=\"1\"}} {}\n{}_bucket{{worker=\"PRIVATE_WORKER\",le=\"+Inf\"}} {}\n{}_count{{worker=\"PRIVATE_WORKER\"}} {}\n{}_sum{{worker=\"PRIVATE_WORKER\"}} {}\n",hist.family,hist.family,unit,hist.family,low,hist.family,count,hist.family,count,hist.family,sum));
        } else {
            let kind = if spec.kind == MetricType::Counter {
                "counter"
            } else {
                "gauge"
            };
            let val = if spec.unit == Unit::Ratio {
                "0.5"
            } else {
                "10"
            };
            text.push_str(&format!(
                "# TYPE {} {}\n{}{{worker=\"PRIVATE_WORKER\"}} {}\n",
                spec.metric, kind, spec.metric, val
            ));
        }
    }
    text
}
fn snapshot(edge: Edge, text: &str) -> PhaseSnapshot {
    let p = ProfileId::Lmcache;
    let probe = probe(p, Some(p.source_revision()), text).unwrap();
    assert!(
        probe.missing_required_metrics.is_empty(),
        "fixture includes actual required native families"
    );
    PhaseSnapshot {
        probe: Some(probe),
        context: BoundaryContext {
            source_key: "PRIVATE_SOURCE".into(),
            run_epoch_key: "PRIVATE_RUN".into(),
            declared_source_revision: Some(p.source_revision().into()),
            epoch: EpochEvidence::Observed("fixture-epoch".into()),
            stage: 1,
            phase: StudyPhase::Measured,
            edge,
            fetch_start_ns: if edge == Edge::Before { 1 } else { 8 },
            fetch_end_ns: if edge == Edge::Before { 2 } else { 9 },
            issuance_or_drain_ns: if edge == Edge::Before { 3 } else { 7 },
        },
    }
}
fn layouts() -> BTreeMap<String, Vec<String>> {
    BTreeMap::from([(FAMILY.into(), vec!["1".into(), "+Inf".into()])])
}
#[test]
fn complete_declared_layout_retains_distinct_count_duration_and_buckets_without_quantiles() {
    let a = snapshot(Edge::Before, &page(false));
    let b = snapshot(Edge::After, &page(true));
    let reports = histogram_windows(&a, &b, &layouts());
    let r = reports.iter().find(|r| r.family == FAMILY).unwrap();
    assert!(r.unknown.is_none());
    assert_eq!(r.count_delta, Some(4.0));
    assert_eq!(r.sum_delta, Some(8.0));
    assert_eq!(r.unit, Unit::Seconds);
    assert_eq!(
        r.buckets.iter().map(|b| b.delta).collect::<Vec<_>>(),
        vec![3.0, 4.0]
    );
    assert!(!r.quantiles_computed);
    assert!(r.buckets[1].positive_infinity && r.buckets[1].upper_bound.is_none());
    assert!(!serde_json::to_string(&reports).unwrap().contains("PRIVATE"));
    let undeclared = histogram_windows(&a, &b, &BTreeMap::new());
    assert!(
        undeclared
            .iter()
            .all(|r| r.unknown == Some(HistogramUnknown::LayoutNotDeclared)
                && r.count_delta.is_none())
    );
}
#[test]
fn cumulative_phase_bucket_conflict_cannot_be_clamped_or_exported_as_a_distribution() {
    let a = snapshot(Edge::Before, &page(false));
    let changed = page(true).replace(
        &format!("{FAMILY}_bucket{{worker=\"PRIVATE_WORKER\",le=\"1\"}} 8"),
        &format!("{FAMILY}_bucket{{worker=\"PRIVATE_WORKER\",le=\"1\"}} 10"),
    );
    let b = snapshot(Edge::After, &changed);
    let reports = histogram_windows(&a, &b, &layouts());
    let r = reports.iter().find(|r| r.family == FAMILY).unwrap();
    assert_eq!(r.unknown, Some(HistogramUnknown::CumulativeConflict));
    assert!(r.count_delta.is_none() && r.sum_delta.is_none() && r.buckets.is_empty());
}
#[test]
fn workers_remain_separate_and_infinite_bucket_must_match_count() {
    let two = |after| {
        let one = page(after);
        format!("{one}{}", one.replace("PRIVATE_WORKER", "PRIVATE_OTHER"))
    };
    let a = snapshot(Edge::Before, &two(false));
    let b = snapshot(Edge::After, &two(true));
    let reports = histogram_windows(&a, &b, &layouts());
    let selected: Vec<_> = reports.iter().filter(|r| r.family == FAMILY).collect();
    assert_eq!(selected.len(), 2);
    assert!(
        selected
            .iter()
            .all(|r| r.unknown.is_none() && r.count_delta == Some(4.0) && r.sum_delta == Some(8.0)),
        "distinct workers must not become one double-counted group"
    );
    assert!(!serde_json::to_string(&reports).unwrap().contains("PRIVATE"));
    let changed = page(true).replace(
        &format!("{FAMILY}_count{{worker=\"PRIVATE_WORKER\"}} 14"),
        &format!("{FAMILY}_count{{worker=\"PRIVATE_WORKER\"}} 15"),
    );
    let reports = histogram_windows(
        &snapshot(Edge::Before, &page(false)),
        &snapshot(Edge::After, &changed),
        &layouts(),
    );
    assert_eq!(
        reports.iter().find(|r| r.family == FAMILY).unwrap().unknown,
        Some(HistogramUnknown::CumulativeConflict)
    );
}
#[test]
fn changed_layout_missing_boundary_series_and_epoch_change_fail_closed() {
    let a = snapshot(Edge::Before, &page(false));
    for changed in [
        page(true).replace("le=\"1\"", "le=\"2\""),
        page(true).replace(
            &format!("{FAMILY}_sum{{worker=\"PRIVATE_WORKER\"}} 28"),
            &format!("{FAMILY}_sum{{worker=\"PRIVATE_WORKER\"}} NaN"),
        ),
    ] {
        let b = snapshot(Edge::After, &changed);
        let reports = histogram_windows(&a, &b, &layouts());
        let r = reports.iter().find(|r| r.family == FAMILY).unwrap();
        assert!(r.unknown.is_some() && r.count_delta.is_none());
    }
    let mut b = snapshot(Edge::After, &page(true));
    b.context.epoch = EpochEvidence::Observed("changed".into());
    let reports = histogram_windows(&a, &b, &layouts());
    assert!(reports
        .iter()
        .all(|r| r.unknown == Some(HistogramUnknown::Window(DeltaUnknown::EpochChanged))));
}
