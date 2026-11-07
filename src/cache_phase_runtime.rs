// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Typed commands for the EXISTING telemetry consumer. No sampler or implicit
//! server epochs. Producers must await before acknowledgements prior to issuance
//! and submit after commands only after joining all in-flight work and settling.
use crate::cache_phase::{Edge, StudyPhase};
use crate::telemetry::{NativeBoundaryConfig, NativeBoundaryOutcome, TelemetrySession};
use std::time::Duration;
use tokio::sync::oneshot;

#[derive(Clone, Copy)]
pub struct PhaseBoundaryTag {
    pub stage: u32,
    pub phase: StudyPhase,
    pub edge: Edge,
}
impl PhaseBoundaryTag {
    pub fn header(self) -> String {
        let phase = match self.phase {
            StudyPhase::Cold => "cold",
            StudyPhase::Populate => "populate",
            StudyPhase::Warm => "warm",
            StudyPhase::Pressure => "pressure",
            StudyPhase::Rehit => "rehit",
        };
        let edge = match self.edge {
            Edge::Before => "before",
            Edge::After => "after",
        };
        format!("cache-stage-{}-{phase}-{edge}", self.stage)
    }
}
// Contains private source/binding/probe: intentionally no Debug/Serialize.
pub struct PhaseBoundaryRequest {
    pub source: String,
    pub options: NativeBoundaryConfig,
    pub tag: PhaseBoundaryTag,
    pub budget_ms: u64,
    pub ack: oneshot::Sender<PhaseBoundaryReply>,
}
pub struct PhaseBoundaryReply {
    pub tag: PhaseBoundaryTag,
    pub outcome: NativeBoundaryOutcome,
    pub budget_expired: bool,
}
impl TelemetrySession {
    /// Dispatch in the existing record consumer's biased command branch. No
    /// fire-and-forget acknowledgement: priority writes finish before ack.
    /// This transports evidence, not a native required-coverage verdict; an HTTP
    /// success or legacy matches cannot substitute for validating the probe.
    pub async fn handle_phase_boundary(&mut self, request: PhaseBoundaryRequest) {
        if request.ack.is_closed() {
            return;
        }
        let tag = request.tag.header();
        let capture = tokio::time::timeout(
            Duration::from_millis(request.budget_ms),
            self.boundary_sample_with_native(&request.source, &request.options, &tag),
        )
        .await;
        let (outcome, budget_expired) = match capture {
            Ok(outcome) => (outcome, false),
            Err(_) => (
                NativeBoundaryOutcome {
                    telemetry: self
                        .note_boundary_timeout_tagged(&tag, request.budget_ms)
                        .await,
                    native: None,
                },
                true,
            ),
        };
        let _ = request.ack.send(PhaseBoundaryReply {
            tag: request.tag,
            outcome,
            budget_expired,
        });
    }
}
