// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::error::RequestError;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::time::Duration;

/// Field-additive schema bump (config, window bounds, first_byte_s land later).
pub const SCHEMA_VERSION_REQUEST: &str = "metrum-ai-bench.request.v3";
pub const SCHEMA_VERSION_SUMMARY: &str = "metrum-ai-bench.summary.v3";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Warmup,
    Measure,
    Drain,
}

impl Phase {
    pub fn for_seq(seq: u64, warmup_requests: u32) -> Self {
        if seq < u64::from(warmup_requests) {
            Phase::Warmup
        } else {
            Phase::Measure
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RequestRecord {
    pub schema_version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub seq: u64,
    pub phase: Phase,
    pub endpoint: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    /// Seconds from the workload epoch at which this request was intended.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduled_offset_s: Option<f64>,
    /// Delay between intended arrival and actual send. Included in
    /// coordinated-omission-corrected latency by summary consumers.
    #[serde(default)]
    pub queue_delay_s: f64,
    /// Seconds from send to completion (monotonic).
    pub latency_s: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttft_s: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_reasoning_s: Option<f64>,
    /// Inter-chunk intervals in seconds (visible tokens only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub itl_s: Vec<f64>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokenized_prompt_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokenized_completion_tokens: Option<u64>,
    #[serde(default)]
    pub usage_missing: bool,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub modality_metrics: std::collections::BTreeMap<String, f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RequestError>,
    pub partial: bool,
}

impl RequestRecord {
    #[allow(clippy::too_many_arguments)]
    pub fn success(
        seq: u64,
        phase: Phase,
        endpoint: String,
        started_at: DateTime<Utc>,
        completed_at: DateTime<Utc>,
        latency: Duration,
        ttft: Option<Duration>,
        first_reasoning: Option<Duration>,
        itl: Vec<Duration>,
        prompt_tokens: u64,
        completion_tokens: u64,
        total_tokens: u64,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION_REQUEST,
            run_id: None,
            seq,
            phase,
            endpoint,
            started_at,
            completed_at,
            scheduled_offset_s: None,
            queue_delay_s: 0.0,
            latency_s: latency.as_secs_f64(),
            ttft_s: ttft.map(|d| d.as_secs_f64()),
            first_reasoning_s: first_reasoning.map(|d| d.as_secs_f64()),
            itl_s: itl.iter().map(|d| d.as_secs_f64()).collect(),
            prompt_tokens,
            completion_tokens,
            total_tokens,
            tokenized_prompt_tokens: None,
            tokenized_completion_tokens: None,
            usage_missing: false,
            modality_metrics: std::collections::BTreeMap::new(),
            error: None,
            partial: false,
        }
    }

    pub fn failed(
        seq: u64,
        phase: Phase,
        endpoint: String,
        started_at: DateTime<Utc>,
        completed_at: DateTime<Utc>,
        latency: Duration,
        error: RequestError,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION_REQUEST,
            run_id: None,
            seq,
            phase,
            endpoint,
            started_at,
            completed_at,
            scheduled_offset_s: None,
            queue_delay_s: 0.0,
            latency_s: latency.as_secs_f64(),
            ttft_s: None,
            first_reasoning_s: None,
            itl_s: Vec::new(),
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            tokenized_prompt_tokens: None,
            tokenized_completion_tokens: None,
            usage_missing: false,
            modality_metrics: std::collections::BTreeMap::new(),
            error: Some(error),
            partial: false,
        }
    }

    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = Some(run_id.into());
        self
    }

    pub fn is_success(&self) -> bool {
        self.error.is_none()
    }

    pub fn with_schedule(mut self, scheduled_offset: Duration, queue_delay: Duration) -> Self {
        self.scheduled_offset_s = Some(scheduled_offset.as_secs_f64());
        self.queue_delay_s = queue_delay.as_secs_f64();
        self
    }

    pub fn corrected_latency_s(&self) -> f64 {
        self.latency_s + self.queue_delay_s
    }

    pub fn tpot_s(&self) -> Option<f64> {
        let ttft = self.ttft_s?;
        let gen = (self.latency_s - ttft).max(0.0);
        if self.completion_tokens <= 1 || gen == 0.0 {
            return None;
        }
        Some(gen / (self.completion_tokens - 1) as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tpot_uses_n_minus_one() {
        let started = Utc::now();
        let latency = Duration::from_millis(500);
        let rec = RequestRecord::success(
            0,
            Phase::Measure,
            "ep".into(),
            started,
            started + chrono::Duration::from_std(latency).unwrap(),
            latency,
            Some(Duration::from_millis(120)),
            None,
            vec![],
            10,
            20,
            30,
        );
        let tpot = rec.tpot_s().expect("tpot");
        let expected = (0.500 - 0.120) / 19.0;
        assert!((tpot - expected).abs() < 1e-9);
    }
}
