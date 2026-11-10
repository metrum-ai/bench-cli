// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Compatible counter ratios over explicit private worker partitions. No fetch,
//! replica/tier inference, mean-of-ratios, or physical cache attribution.
use crate::cache_metrics::{Role, Unit};
use crate::cache_phase::{per_series_deltas, Continuity, DeltaUnknown, PhaseSnapshot, StudyPhase};
use crate::telemetry::MetricType;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

type Labels = BTreeMap<String, String>;
#[derive(Clone, Copy)]
pub enum CoverageMode {
    Optional,
    Required,
}
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PartitionEvidence {
    SingleSeries,
    OperatorDeclaredNonOverlapping,
}
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrafficScope {
    SyntheticExclusive,
    OperatorDeclaredExclusive,
    SharedOrUnknown,
}
/// Private selectors/declarations; deliberately neither Debug nor Serialize.
pub struct RateRequest<'a> {
    pub hit_metric: &'a str,
    pub query_metric: &'a str,
    pub expected_workers: &'a [Labels],
    pub partition: PartitionEvidence,
    pub traffic: TrafficScope,
    pub coverage: CoverageMode,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RateUnknown {
    Boundary(DeltaUnknown),
    InvalidDefinition,
    MissingPartitionDeclaration,
    DuplicatePartition,
    UnexpectedPartition,
    MissingSeries,
    InvalidSeries,
    Arithmetic,
    ZeroDenominator,
    ContradictoryCounters,
    SharedTraffic,
}
#[derive(Debug, Serialize)]
pub struct CounterRate {
    pub schema_version: &'static str,
    pub phase: StudyPhase,
    pub stage: u32,
    pub continuity: Continuity,
    pub partition_evidence: PartitionEvidence,
    pub traffic_scope: TrafficScope,
    pub expected_workers: usize,
    pub covered_workers: usize,
    pub unit: Option<Unit>,
    /// Decimal strings retain exact sums above the binary64 integer limit.
    pub hit_delta: Option<String>,
    pub query_delta: Option<String>,
    pub ratio: Option<f64>,
    pub unknown: Option<RateUnknown>,
    pub attribution: &'static str,
}
#[derive(Debug)]
pub struct RequiredCoverageError {
    pub report: Box<CounterRate>,
}
impl std::fmt::Display for RequiredCoverageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "required counter-rate coverage unavailable: {:?}",
            self.report.unknown
        )
    }
}
impl std::error::Error for RequiredCoverageError {}

fn integer(value: f64) -> Option<u128> {
    (value.is_finite() && (0.0..=9_007_199_254_740_991.0).contains(&value) && value.fract() == 0.0)
        .then_some(value as u128)
}

pub fn counter_rate(
    before: &PhaseSnapshot,
    after: &PhaseSnapshot,
    request: &RateRequest<'_>,
) -> Result<CounterRate, RequiredCoverageError> {
    let deltas = per_series_deltas(before, after);
    let mut report=CounterRate {schema_version:"cache-phase-rate.v1",phase:deltas.phase,stage:deltas.stage,continuity:deltas.continuity,
        partition_evidence:request.partition,traffic_scope:request.traffic,expected_workers:request.expected_workers.len(),covered_workers:0,
        unit:None,hit_delta:None,query_delta:None,ratio:None,unknown:None,
        attribution:"endpoint differences; explicit partition/traffic declarations are conditional; sample freshness and physical attribution not verified"};
    let mut calculate = || -> Result<(Unit, usize, u128, u128), RateUnknown> {
        if let Some(reason) = deltas.window_unknown {
            return Err(RateUnknown::Boundary(reason));
        }
        if matches!(request.traffic, TrafficScope::SharedOrUnknown) {
            return Err(RateUnknown::SharedTraffic);
        }
        if request.expected_workers.is_empty() || request.expected_workers.len() > 100_000 {
            return Err(RateUnknown::MissingPartitionDeclaration);
        }
        if request.expected_workers.len() > 1
            && matches!(request.partition, PartitionEvidence::SingleSeries)
        {
            return Err(RateUnknown::MissingPartitionDeclaration);
        }
        let workers: BTreeSet<_> = request.expected_workers.iter().cloned().collect();
        if workers.len() != request.expected_workers.len() {
            return Err(RateUnknown::DuplicatePartition);
        }
        let (Some(p), Some(q)) = (&before.probe, &after.probe) else {
            return Err(RateUnknown::MissingSeries);
        };
        let hit = p
            .profile
            .metrics()
            .iter()
            .find(|s| s.metric == request.hit_metric)
            .ok_or(RateUnknown::InvalidDefinition)?;
        let query = p
            .profile
            .metrics()
            .iter()
            .find(|s| s.metric == request.query_metric)
            .ok_or(RateUnknown::InvalidDefinition)?;
        if hit.role != Role::Hit
            || query.role != Role::Query
            || hit.unit != query.unit
            || !matches!(hit.unit, Unit::Tokens | Unit::Requests)
            || hit.kind != MetricType::Counter
            || query.kind != MetricType::Counter
        {
            return Err(RateUnknown::InvalidDefinition);
        }
        let keys: BTreeSet<_> = p
            .observations
            .iter()
            .chain(&q.observations)
            .map(|r| (r.spec.metric.to_owned(), r.labels.clone()))
            .collect();
        // Same complete-key ordering as per_series_deltas; reuse its validated
        // differences, never independently subtract/clamp/reset counters here.
        if keys.len() != deltas.series.len() {
            return Err(RateUnknown::Arithmetic);
        }
        let paired: BTreeMap<_, _> = keys.into_iter().zip(deltas.series.iter()).collect();
        if paired.keys().any(|(name, labels)| {
            (name == request.hit_metric || name == request.query_metric)
                && !workers.contains(labels)
        }) {
            return Err(RateUnknown::UnexpectedPartition);
        }
        let mut hits = 0_u128;
        let mut queries = 0_u128;
        let mut covered = 0;
        let mut first_unknown = None;
        report.unit = Some(hit.unit);
        for labels in &workers {
            let pair = || -> Result<(u128, u128), RateUnknown> {
                let h = paired
                    .get(&(request.hit_metric.to_owned(), labels.clone()))
                    .ok_or(RateUnknown::MissingSeries)?;
                let q = paired
                    .get(&(request.query_metric.to_owned(), labels.clone()))
                    .ok_or(RateUnknown::MissingSeries)?;
                if h.unknown.is_some() || q.unknown.is_some() {
                    return Err(RateUnknown::InvalidSeries);
                }
                let h = integer(h.delta.ok_or(RateUnknown::InvalidSeries)?)
                    .ok_or(RateUnknown::Arithmetic)?;
                let q = integer(q.delta.ok_or(RateUnknown::InvalidSeries)?)
                    .ok_or(RateUnknown::Arithmetic)?;
                if h > q {
                    return Err(RateUnknown::ContradictoryCounters);
                }
                Ok((h, q))
            };
            match pair() {
                Ok((h, q)) => {
                    hits = hits.checked_add(h).ok_or(RateUnknown::Arithmetic)?;
                    queries = queries.checked_add(q).ok_or(RateUnknown::Arithmetic)?;
                    covered += 1;
                    report.covered_workers = covered;
                }
                Err(reason) => {
                    first_unknown.get_or_insert(reason);
                }
            }
        }
        if let Some(reason) = first_unknown {
            return Err(reason);
        }
        Ok((hit.unit, covered, hits, queries))
    };
    match calculate() {
        Ok((unit, covered, hits, queries)) => {
            report.unit = Some(unit);
            report.covered_workers = covered;
            report.hit_delta = Some(hits.to_string());
            report.query_delta = Some(queries.to_string());
            if queries == 0 {
                report.unknown = Some(RateUnknown::ZeroDenominator);
            } else {
                report.ratio = Some(hits as f64 / queries as f64);
            }
        }
        Err(reason) => report.unknown = Some(reason),
    }
    if report.unknown.is_some() && matches!(request.coverage, CoverageMode::Required) {
        Err(RequiredCoverageError {
            report: Box::new(report),
        })
    } else {
        Ok(report)
    }
}
