// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Explicit measured-window selection. No cold/warm physical state inference.
use crate::{
    cache_metric_capture::NativeMetricArgs, cache_phase::*, cache_phase_runtime::*, telemetry::*,
};
use anyhow::{ensure, Result};
use serde_json::{json, Value};

#[derive(clap::Args, Clone, Default)]
pub struct PhaseMetricArgs {
    #[arg(long,requires_all=["telemetry","ndjson","cache_metrics_profile","cache_metrics_url"],help="Opt-in measured-window native analysis on this exact configured telemetry source")]
    pub cache_phase_source: Option<String>,
    #[arg(
        long,
        requires = "cache_phase_source",
        help = "Explicit operator-declared continuity identifier; not an observed process/manager epoch"
    )]
    pub cache_phase_epoch: Option<String>,
    #[arg(
        long,
        requires = "cache_phase_source",
        help = "Fail when native counter coverage is unavailable; requires explicit revision and continuity declaration"
    )]
    pub cache_phase_required: bool,
    #[arg(
        long,
        default_value_t = 0,
        help = "Opt-in native terminal settling after complete request drain (0..10000 ms)"
    )]
    pub cache_phase_settle_ms: u64,
}
impl std::fmt::Debug for PhaseMetricArgs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhaseMetricArgs")
            .field("configured", &self.cache_phase_source.is_some())
            .field("required", &self.cache_phase_required)
            .finish_non_exhaustive()
    }
}
// Private source/continuity identity, deliberately no Debug/Serialize.
pub struct PhaseMetricPlan {
    pub source: String,
    pub options: NativeBoundaryConfig,
    pub epoch: Option<String>,
    pub required: bool,
    pub settle_ms: u64,
}
impl PhaseMetricArgs {
    pub fn resolve(
        &self,
        cfg: Option<&TelemetryConfig>,
        has_ndjson: bool,
        native: &NativeMetricArgs,
    ) -> Result<Option<PhaseMetricPlan>> {
        ensure!(
            self.cache_phase_settle_ms <= 10_000,
            "native phase settling must be <=10000 ms"
        );
        let Some(name) = &self.cache_phase_source else {
            ensure!(
                self.cache_phase_epoch.is_none()
                    && !self.cache_phase_required
                    && self.cache_phase_settle_ms == 0,
                "native phase controls require --cache-phase-source"
            );
            return Ok(None);
        };
        ensure!(has_ndjson, "native phase analysis requires --ndjson");
        native.validate()?;
        let cfg = cfg.ok_or_else(|| {
            anyhow::anyhow!("native phase analysis requires configured telemetry")
        })?;
        let source = cfg
            .sources
            .iter()
            .find(|s| s.name == *name)
            .ok_or_else(|| anyhow::anyhow!("native phase source is not configured"))?;
        let profile = native.cache_metrics_profile.ok_or_else(|| {
            anyhow::anyhow!("native phase analysis requires explicit native profile")
        })?;
        let url = native.cache_metrics_url.as_deref().ok_or_else(|| {
            anyhow::anyhow!("native phase analysis requires explicit native endpoint")
        })?;
        ensure!(
            reqwest::Url::parse(&source.url)? == reqwest::Url::parse(url)?,
            "native phase endpoint must match the selected configured source"
        );
        ensure!(
            !source.insecure_tls,
            "native phase transport does not support insecure TLS"
        );
        if let Some(epoch) = &self.cache_phase_epoch {
            ensure!(
                !epoch.trim().is_empty() && epoch.len() <= 512,
                "continuity declaration must be nonempty and <=512 bytes"
            );
        }
        if self.cache_phase_required {
            ensure!(self.cache_phase_epoch.is_some(),"required native coverage needs an explicit continuity declaration (not executable proof)");
            ensure!(
                native.cache_metrics_source_revision.as_deref() == Some(profile.source_revision()),
                "required native coverage needs the exact supported source revision"
            );
        }
        Ok(Some(PhaseMetricPlan {
            source: name.clone(),
            options: NativeBoundaryConfig {
                profile,
                source_revision: native.cache_metrics_source_revision.clone(),
            },
            epoch: self.cache_phase_epoch.clone(),
            required: self.cache_phase_required,
            settle_ms: self.cache_phase_settle_ms,
        }))
    }
}
impl PhaseMetricPlan {
    pub fn request(
        &self,
        tag: PhaseBoundaryTag,
        budget_ms: u64,
    ) -> (
        PhaseBoundaryRequest,
        tokio::sync::oneshot::Receiver<PhaseBoundaryReply>,
    ) {
        let (ack, reply) = tokio::sync::oneshot::channel();
        (
            PhaseBoundaryRequest {
                source: self.source.clone(),
                options: NativeBoundaryConfig {
                    profile: self.options.profile,
                    source_revision: self.options.source_revision.clone(),
                },
                tag,
                budget_ms,
                ack,
            },
            reply,
        )
    }
    pub fn covered(&self, reply: &PhaseBoundaryReply) -> bool {
        !reply.budget_expired
            && reply.outcome.native.as_ref().is_some_and(|cap| {
                cap.failure.is_none()
                    && cap.probe.as_ref().is_some_and(|p| {
                        p.missing_required_metrics.is_empty()
                            && p.observations.iter().all(|o| o.unknown.is_none())
                    })
            })
    }
    pub fn report(
        &self,
        run_id: &str,
        before: Option<PhaseBoundaryReply>,
        after: Option<PhaseBoundaryReply>,
        issued_ns: Option<u64>,
        drained_ns: Option<u64>,
    ) -> Value {
        let (Some(before), Some(after), Some(issued), Some(drained)) =
            (before, after, issued_ns, drained_ns)
        else {
            return json!({"schema_version":"cache-phase-analysis.v1","unknown":"capture_unavailable","physical_cache_state":"unknown"});
        };
        let build = |reply: PhaseBoundaryReply, anchor: u64| -> Option<PhaseSnapshot> {
            let capture = reply.outcome.native?;
            Some(PhaseSnapshot {
                context: BoundaryContext {
                    source_key: self.source.clone(),
                    run_epoch_key: run_id.into(),
                    declared_source_revision: self.options.source_revision.clone(),
                    epoch: self.epoch.as_ref().map_or(EpochEvidence::Unavailable, |s| {
                        EpochEvidence::OperatorDeclared(s.clone())
                    }),
                    stage: reply.tag.stage,
                    phase: reply.tag.phase,
                    edge: reply.tag.edge,
                    fetch_start_ns: capture.fetch_start_ns,
                    fetch_end_ns: capture.fetch_end_ns,
                    issuance_or_drain_ns: anchor,
                },
                probe: capture.probe,
            })
        };
        match (build(before, issued), build(after, drained)) {
            (Some(a), Some(b)) => {
                json!({"schema_version":"cache-phase-analysis.v1","delta":per_series_deltas(&a,&b),"histograms":crate::cache_histogram::histogram_windows(&a,&b,&std::collections::BTreeMap::new()),"histogram_layout_evidence":"not_declared; complete distributions and quantiles not certified","physical_cache_state":"unknown","sample_freshness":"unknown","workload_exclusivity":"unknown","settle_ms":self.settle_ms})
            }
            _ => {
                json!({"schema_version":"cache-phase-analysis.v1","unknown":"capture_unavailable","physical_cache_state":"unknown"})
            }
        }
    }
}
