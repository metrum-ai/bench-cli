// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Pure phase arithmetic over #417 observations. This module does not fetch or
//! schedule samples. Integrate with #312 boundaries, never a new sampling loop.

use crate::cache_metrics::{Probe, Unit};
use crate::telemetry::MetricType;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Before,
    After,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StudyPhase {
    Measured,
    Cold,
    Populate,
    Warm,
    Pressure,
    Rehit,
}

// Private identity/evidence; deliberately not Debug or Serialize.
pub enum EpochEvidence {
    Observed(String),
    OperatorDeclared(String),
    Unavailable,
}
pub struct BoundaryContext {
    pub source_key: String,
    pub run_epoch_key: String,
    pub declared_source_revision: Option<String>,
    pub epoch: EpochEvidence,
    pub stage: u32,
    pub phase: StudyPhase,
    pub edge: Edge,
    pub fetch_start_ns: u64,
    pub fetch_end_ns: u64,
    /// Before: first issuance; After: completion of drain/declared settling.
    pub issuance_or_drain_ns: u64,
}
pub struct PhaseSnapshot {
    pub context: BoundaryContext,
    pub probe: Option<Probe>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeltaUnknown {
    CaptureUnavailable,
    SourceChanged,
    RunEpochChanged,
    ProfileChanged,
    RevisionUnsupported,
    PhaseChanged,
    BoundaryOrder,
    WindowOverlap,
    LateBaseline,
    EarlyTerminal,
    EpochUnavailable,
    EpochChanged,
    MissingStart,
    MissingEnd,
    DuplicateIdentity,
    InvalidBoundary,
    DescriptorChanged,
    GaugeNotCounter,
    CounterDecrease,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Continuity {
    ObservedMarker,
    OperatorDeclared,
    Unknown,
}
#[derive(Serialize)]
pub struct SeriesDelta {
    /// Scoped to this report; not a stable worker identity.
    pub series_id: usize,
    pub metric: String,
    pub unit: Unit,
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub delta: Option<f64>,
    pub unknown: Option<DeltaUnknown>,
}
#[derive(Serialize)]
pub struct PhaseDeltaReport {
    pub schema_version: &'static str,
    pub phase: StudyPhase,
    pub stage: u32,
    pub continuity: Continuity,
    pub window_unknown: Option<DeltaUnknown>,
    pub attribution: &'static str,
    pub series: Vec<SeriesDelta>,
}

type Key = (String, BTreeMap<String, String>);
fn rows(
    probe: &Probe,
) -> (
    BTreeMap<Key, &crate::cache_metrics::Observation>,
    BTreeSet<Key>,
) {
    let mut rows = BTreeMap::new();
    let mut duplicates = BTreeSet::new();
    for row in &probe.observations {
        let key = (row.spec.metric.to_owned(), row.labels.clone());
        if rows.insert(key.clone(), row).is_some() {
            duplicates.insert(key);
        }
    }
    (rows, duplicates)
}

fn validate_window(a: &PhaseSnapshot, b: &PhaseSnapshot) -> (Continuity, Option<DeltaUnknown>) {
    let (x, y) = (&a.context, &b.context);
    let fail = |reason| (Continuity::Unknown, Some(reason));
    if x.run_epoch_key.is_empty() || x.run_epoch_key != y.run_epoch_key {
        return fail(DeltaUnknown::RunEpochChanged);
    }
    if x.source_key.is_empty() || x.source_key != y.source_key {
        return fail(DeltaUnknown::SourceChanged);
    }
    if x.stage != y.stage || x.phase != y.phase {
        return fail(DeltaUnknown::PhaseChanged);
    }
    if x.edge != Edge::Before || y.edge != Edge::After {
        return fail(DeltaUnknown::BoundaryOrder);
    }
    if x.fetch_start_ns > x.fetch_end_ns
        || y.fetch_start_ns > y.fetch_end_ns
        || x.fetch_end_ns >= y.fetch_start_ns
    {
        return fail(DeltaUnknown::WindowOverlap);
    }
    if x.fetch_end_ns > x.issuance_or_drain_ns {
        return fail(DeltaUnknown::LateBaseline);
    }
    if y.fetch_start_ns < y.issuance_or_drain_ns {
        return fail(DeltaUnknown::EarlyTerminal);
    }
    let (Some(p), Some(q)) = (&a.probe, &b.probe) else {
        return fail(DeltaUnknown::CaptureUnavailable);
    };
    if p.profile != q.profile {
        return fail(DeltaUnknown::ProfileChanged);
    }
    if x.declared_source_revision.as_deref() != Some(p.profile.source_revision())
        || x.declared_source_revision != y.declared_source_revision
    {
        return fail(DeltaUnknown::RevisionUnsupported);
    }
    if !p.missing_required_metrics.is_empty() || !q.missing_required_metrics.is_empty() {
        return fail(DeltaUnknown::InvalidBoundary);
    }
    match (&x.epoch, &y.epoch) {
        (EpochEvidence::Observed(a), EpochEvidence::Observed(b)) if !a.is_empty() && a == b => {
            (Continuity::ObservedMarker, None)
        }
        (EpochEvidence::OperatorDeclared(a), EpochEvidence::OperatorDeclared(b))
            if !a.is_empty() && a == b =>
        {
            (Continuity::OperatorDeclared, None)
        }
        (EpochEvidence::Unavailable, _) | (_, EpochEvidence::Unavailable) => {
            (Continuity::Unknown, Some(DeltaUnknown::EpochUnavailable))
        }
        _ => fail(DeltaUnknown::EpochChanged),
    }
}

/// No sum, ratio, bandwidth or quantile is inferred here. A valid delta remains
/// reported endpoint evidence; even an observed marker cannot exclude other
/// traffic or detect every internal manager reset hidden by an exporter.
pub fn per_series_deltas(a: &PhaseSnapshot, b: &PhaseSnapshot) -> PhaseDeltaReport {
    let (continuity, window_unknown) = validate_window(a, b);
    let (left, left_duplicates) = a.probe.as_ref().map(rows).unwrap_or_default();
    let (right, right_duplicates) = b.probe.as_ref().map(rows).unwrap_or_default();
    let keys: BTreeSet<_> = left.keys().chain(right.keys()).cloned().collect();
    let series = keys
        .into_iter()
        .enumerate()
        .map(|(index, key)| {
            let start = left.get(&key).copied();
            let end = right.get(&key).copied();
            let spec = start
                .or(end)
                .expect("key belongs to at least one boundary")
                .spec;
            let mut reason = window_unknown;
            if reason.is_none() {
                reason = match (start, end) {
                    (None, _) => Some(DeltaUnknown::MissingStart),
                    (_, None) => Some(DeltaUnknown::MissingEnd),
                    (Some(s), Some(e)) => {
                        if left_duplicates.contains(&key) || right_duplicates.contains(&key) {
                            Some(DeltaUnknown::DuplicateIdentity)
                        } else if s.spec.kind != e.spec.kind
                            || s.spec.unit != e.spec.unit
                            || s.spec.role != e.spec.role
                        {
                            Some(DeltaUnknown::DescriptorChanged)
                        } else if s.unknown.is_some()
                            || e.unknown.is_some()
                            || !s.value.is_some_and(|v| v.is_finite() && v >= 0.0)
                            || !e.value.is_some_and(|v| v.is_finite() && v >= 0.0)
                        {
                            Some(DeltaUnknown::InvalidBoundary)
                        } else if !matches!(
                            spec.kind,
                            MetricType::Counter | MetricType::HistogramBucket
                        ) {
                            Some(DeltaUnknown::GaugeNotCounter)
                        } else if e.value < s.value {
                            Some(DeltaUnknown::CounterDecrease)
                        } else {
                            None
                        }
                    }
                };
            }
            let delta = if reason.is_none() {
                Some(end.unwrap().value.unwrap() - start.unwrap().value.unwrap())
            } else {
                None
            };
            SeriesDelta {
                series_id: index + 1,
                metric: key.0,
                unit: spec.unit,
                start: start.and_then(|s| s.value),
                end: end.and_then(|s| s.value),
                delta,
                unknown: reason,
            }
        })
        .collect();
    PhaseDeltaReport {schema_version:"cache-phase-delta.v1",phase:a.context.phase,stage:a.context.stage,continuity,window_unknown,
        attribution:"endpoint-reported; workload contamination/freshness unknown; no tier/replica sum or physical validation",series}
}
