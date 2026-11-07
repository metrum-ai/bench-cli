// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use metrum_ai_bench::cache_metrics::{probe, ProfileId};
use metrum_ai_bench::cache_phase::*;

fn snapshot(edge: Edge, query: u64, hit: u64, worker: &str) -> PhaseSnapshot {
    let p = ProfileId::VllmApc;
    let page=format!("# TYPE vllm:prefix_cache_queries_total counter\nvllm:prefix_cache_queries_total{{worker=\"{worker}\"}} {query}\n# TYPE vllm:prefix_cache_hits_total counter\nvllm:prefix_cache_hits_total{{worker=\"{worker}\"}} {hit}\n# TYPE vllm:kv_cache_usage_perc gauge\nvllm:kv_cache_usage_perc{{worker=\"{worker}\"}} 0.5\n");
    PhaseSnapshot {
        context: BoundaryContext {
            source_key: "PRIVATE_SOURCE".into(),
            run_epoch_key: "PRIVATE_RUN".into(),
            declared_source_revision: Some(p.source_revision().into()),
            epoch: EpochEvidence::Observed("PRIVATE_EPOCH".into()),
            stage: 1,
            phase: StudyPhase::Warm,
            edge,
            fetch_start_ns: if edge == Edge::Before { 1 } else { 8 },
            fetch_end_ns: if edge == Edge::Before { 2 } else { 9 },
            issuance_or_drain_ns: if edge == Edge::Before { 3 } else { 7 },
        },
        probe: Some(probe(p, Some(p.source_revision()), &page).unwrap()),
    }
}

#[test]
fn lifetime_baseline_and_warmup_hits_do_not_become_phase_hits_or_gauge_deltas() {
    let start = snapshot(Edge::Before, 100, 90, "PRIVATE_WORKER");
    let end = snapshot(Edge::After, 110, 90, "PRIVATE_WORKER");
    let result = per_series_deltas(&start, &end);
    assert!(result.window_unknown.is_none());
    assert_eq!(result.continuity, Continuity::ObservedMarker);
    assert_eq!(
        result
            .series
            .iter()
            .find(|row| row.metric == "vllm:prefix_cache_queries_total")
            .unwrap()
            .delta,
        Some(10.0)
    );
    assert_eq!(
        result
            .series
            .iter()
            .find(|row| row.metric == "vllm:prefix_cache_hits_total")
            .unwrap()
            .delta,
        Some(0.0)
    );
    let gauge = result
        .series
        .iter()
        .find(|row| row.metric == "vllm:kv_cache_usage_perc")
        .unwrap();
    assert_eq!(gauge.start, Some(0.5));
    assert_eq!(gauge.end, Some(0.5));
    assert!(gauge.delta.is_none());
    assert_eq!(gauge.unknown, Some(DeltaUnknown::GaugeNotCounter));
    assert!(!serde_json::to_string(&result).unwrap().contains("PRIVATE_"));
}

#[test]
fn counter_decrease_epoch_loss_and_identity_changes_cannot_be_clamped_or_joined() {
    let start = snapshot(Edge::Before, 100, 90, "a");
    let end = snapshot(Edge::After, 50, 40, "a");
    let result = per_series_deltas(&start, &end);
    assert!(result
        .series
        .iter()
        .filter(|row| row.metric.ends_with("_total"))
        .all(|row| row.delta.is_none() && row.unknown == Some(DeltaUnknown::CounterDecrease)));
    for kind in 0..8 {
        let mut end = snapshot(Edge::After, 110, 95, if kind == 7 { "b" } else { "a" });
        let reason = match kind {
            0 => {
                end.context.epoch = EpochEvidence::Unavailable;
                Some(DeltaUnknown::EpochUnavailable)
            }
            1 => {
                end.context.epoch = EpochEvidence::Observed("new-process".into());
                Some(DeltaUnknown::EpochChanged)
            }
            2 => {
                end.context.source_key = "other".into();
                Some(DeltaUnknown::SourceChanged)
            }
            3 => {
                end.context.run_epoch_key = "other".into();
                Some(DeltaUnknown::RunEpochChanged)
            }
            4 => {
                end.context.fetch_start_ns = 1;
                Some(DeltaUnknown::WindowOverlap)
            }
            5 => {
                end.context.issuance_or_drain_ns = 10;
                Some(DeltaUnknown::EarlyTerminal)
            }
            6 => {
                end.probe = None;
                Some(DeltaUnknown::CaptureUnavailable)
            }
            _ => None,
        };
        let result = per_series_deltas(&start, &end);
        assert_eq!(result.window_unknown, reason);
        assert!(result.series.iter().all(|row| row.delta.is_none()));
        if kind == 7 {
            assert_eq!(result.series.len(), 6);
            assert!(result.series.iter().all(|row| matches!(
                row.unknown,
                Some(DeltaUnknown::MissingStart | DeltaUnknown::MissingEnd)
            )));
        }
    }
}

#[test]
fn declared_continuity_is_labelled_and_late_baseline_fails_closed() {
    let mut start = snapshot(Edge::Before, 10, 5, "a");
    let mut end = snapshot(Edge::After, 20, 10, "a");
    start.context.epoch = EpochEvidence::OperatorDeclared("declared".into());
    end.context.epoch = EpochEvidence::OperatorDeclared("declared".into());
    assert_eq!(
        per_series_deltas(&start, &end).continuity,
        Continuity::OperatorDeclared
    );
    start.context.issuance_or_drain_ns = 1;
    let result = per_series_deltas(&start, &end);
    assert_eq!(result.window_unknown, Some(DeltaUnknown::LateBaseline));
    assert!(result.series.iter().all(|row| row.delta.is_none()));
}
