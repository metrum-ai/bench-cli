// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Declared-layout histogram component/window checks, never quantiles or tier sums.
use crate::{
    cache_metrics::{HistogramPart, Unit},
    cache_phase::*,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
type Key = (String, BTreeMap<String, String>);
type Components<'a> = BTreeMap<Key, (HistogramPart, &'a SeriesDelta)>;
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistogramUnknown {
    Window(DeltaUnknown),
    LayoutNotDeclared,
    InvalidLayout,
    LayoutChanged,
    MissingComponent,
    InvalidComponent,
    CumulativeConflict,
}
#[derive(Serialize)]
pub struct BucketDelta {
    pub upper_bound: Option<f64>,
    pub positive_infinity: bool,
    pub delta: f64,
}
#[derive(Serialize)]
pub struct HistogramWindow {
    pub group_id: usize,
    pub family: String,
    pub unit: Unit,
    pub count_delta: Option<f64>,
    pub sum_delta: Option<f64>,
    pub buckets: Vec<BucketDelta>,
    pub unknown: Option<HistogramUnknown>,
    pub completeness_scope: &'static str,
    pub quantiles_computed: bool,
}
fn bound(text: &str) -> Option<f64> {
    if text == "+Inf" || text == "Inf" {
        return Some(f64::INFINITY);
    }
    text.parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
}
/// Expected layouts are explicit declarations, not inferred from observed `le`
/// labels. Every other label stays in the private group identity. Output IDs are
/// report-local and no raw label strings/worker/source identity are projected.
pub fn histogram_windows(
    a: &PhaseSnapshot,
    b: &PhaseSnapshot,
    layouts: &BTreeMap<String, Vec<String>>,
) -> Vec<HistogramWindow> {
    let delta = per_series_deltas(a, b);
    let observations = a
        .probe
        .iter()
        .chain(b.probe.iter())
        .flat_map(|p| p.observations.iter());
    let mut groups: BTreeMap<Key, (Unit, Components<'_>)> = BTreeMap::new();
    let keys: BTreeSet<Key> = observations
        .clone()
        .map(|r| (r.spec.metric.to_owned(), r.labels.clone()))
        .collect();
    let paired: BTreeMap<_, _> = keys.into_iter().zip(delta.series.iter()).collect();
    for row in observations {
        if let Some(hist) = row.spec.histogram {
            let mut labels = row.labels.clone();
            labels.remove("le");
            // Index each component directly into its complete private group,
            // avoiding repeated all-row scans/label clones per worker/family.
            let group = groups
                .entry((hist.family.into(), labels))
                .or_insert_with(|| (hist.unit, Components::new()));
            let key = (row.spec.metric.into(), row.labels.clone());
            if let Some(series) = paired.get(&key) {
                group.1.insert(key, (hist.part, *series));
            }
        }
    }
    groups.into_iter().enumerate().map(|(index,((family,_labels),(unit,parts)))| {
        let mut report=HistogramWindow{group_id:index+1,family:family.clone(),unit,count_delta:None,sum_delta:None,buckets:Vec::new(),unknown:None,completeness_scope:"explicit declared layout and endpoint components only; freshness/manager continuity not independently verified",quantiles_computed:false};
        let calculate=||->Result<(f64,f64,Vec<BucketDelta>),HistogramUnknown> {
            if let Some(reason)=delta.window_unknown {return Err(HistogramUnknown::Window(reason));}
            let expected=layouts.get(&family).ok_or(HistogramUnknown::LayoutNotDeclared)?;
            if expected.len()>256 || expected.iter().any(|s|s.len()>128) {return Err(HistogramUnknown::InvalidLayout);}
            let bounds:Vec<_>=expected.iter().map(|s|bound(s).ok_or(HistogramUnknown::InvalidLayout)).collect::<Result<_,_>>()?;
            if bounds.is_empty() || bounds.last()!=Some(&f64::INFINITY) || bounds.windows(2).any(|w|w[0]>=w[1]) {return Err(HistogramUnknown::InvalidLayout);}
            let mut buckets=BTreeMap::new();let mut count=None;let mut sum=None;
            for ((_metric,row_labels),(part,series)) in &parts {
                let le=row_labels.get("le").cloned();
                if series.unknown.is_some() || series.delta.is_none() {return Err(HistogramUnknown::InvalidComponent);}
                match part {
                    HistogramPart::Bucket=>{buckets.insert(le.ok_or(HistogramUnknown::InvalidComponent)?,*series);}
                    HistogramPart::Count=>{if le.is_some() || count.replace(*series).is_some() {return Err(HistogramUnknown::InvalidComponent);}}
                    HistogramPart::Sum=>{if le.is_some() || sum.replace(*series).is_some() {return Err(HistogramUnknown::InvalidComponent);}}
                }
            }
            if buckets.keys().collect::<BTreeSet<_>>()!=expected.iter().collect::<BTreeSet<_>>() {return Err(HistogramUnknown::LayoutChanged);}
            let (count,sum)=(count.ok_or(HistogramUnknown::MissingComponent)?,sum.ok_or(HistogramUnknown::MissingComponent)?);
            let mut previous=(0.0,0.0,0.0);let mut out=Vec::new();
            for (label,upper) in expected.iter().zip(&bounds) {
                let row=buckets.get(label).ok_or(HistogramUnknown::MissingComponent)?;
                let current=(row.start.ok_or(HistogramUnknown::InvalidComponent)?,row.end.ok_or(HistogramUnknown::InvalidComponent)?,row.delta.ok_or(HistogramUnknown::InvalidComponent)?);
                if current.0<previous.0 || current.1<previous.1 || current.2<previous.2 {return Err(HistogramUnknown::CumulativeConflict);}
                previous=current;out.push(BucketDelta{upper_bound:upper.is_finite().then_some(*upper),positive_infinity:upper.is_infinite(),delta:current.2});
            }
            if Some(previous.0)!=count.start || Some(previous.1)!=count.end || Some(previous.2)!=count.delta {return Err(HistogramUnknown::CumulativeConflict);}
            let (count,sum)=(count.delta.ok_or(HistogramUnknown::InvalidComponent)?,sum.delta.ok_or(HistogramUnknown::InvalidComponent)?);
            if count==0.0 && sum!=0.0 {return Err(HistogramUnknown::CumulativeConflict);}
            Ok((count,sum,out))
        };
        match calculate() {Ok((count,sum,buckets))=>{report.count_delta=Some(count);report.sum_delta=Some(sum);report.buckets=buckets;},Err(reason)=>report.unknown=Some(reason)}
        report
    }).collect()
}
