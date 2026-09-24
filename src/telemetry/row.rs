// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Schema version for strategic telemetry NDJSON rows.
pub const TELEMETRY_SCHEMA_VERSION: &str = "metrum-ai-bench-cli.telemetry.v1";

/// Prometheus / OpenMetrics metric type as recorded at ingest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricType {
    Counter,
    Gauge,
    HistogramBucket,
    Summary,
    Unknown,
}

/// Warmup vs measured phase for a stage window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseKind {
    Warmup,
    Measure,
}

/// One NDJSON line. Discriminated by `kind`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Row {
    Run(RunRow),
    Stage(StageRow),
    Telemetry(TelemetryRow),
    Request(RequestRow),
    ScrapeError(ScrapeErrorRow),
    Summary(SummaryRow),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetrySourceStamp {
    pub name: String,
    pub url: String,
    pub interval_ms: u64,
    /// Clock offset in milliseconds: `Date` header (if present) minus local.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clock_offset_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_series: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRow {
    pub run_id: String,
    /// ISO 8601 UTC wall clock at epoch capture.
    pub t0_wall: String,
    pub tool_version: String,
    pub schema_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sut: Option<Value>,
    pub config: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub telemetry_sources: Vec<TelemetrySourceStamp>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageRow {
    pub run_id: String,
    pub stage: f64,
    pub load: f64,
    pub phase: PhaseKind,
    pub t_start_ns: u64,
    pub t_end_ns: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryRow {
    pub run_id: String,
    pub t_ns: u64,
    pub src: String,
    pub metric: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: BTreeMap<String, String>,
    pub value: f64,
    pub unit: String,
    pub mtype: MetricType,
    pub scrape_ms: f64,
    /// Present only when a units scale != 1 was applied at ingest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestRow {
    pub run_id: String,
    pub seq: u64,
    pub stage: f64,
    pub warmup: bool,
    pub t_sched_ns: u64,
    pub t_sent_ns: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub t_first_ns: Option<u64>,
    pub t_done_ns: u64,
    pub success: bool,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub latency_s: f64,
    pub queue_delay_s: f64,
    pub service_latency_s: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttft_s: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Last-seen included metric values at request completion (sugar, not truth).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry_at_done: Option<BTreeMap<String, f64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrapeErrorRow {
    pub run_id: String,
    pub t_ns: u64,
    pub src: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryRow {
    pub run_id: String,
    pub partial: bool,
    pub dropped_telemetry_rows: u64,
    pub request_rows: u64,
    pub telemetry_rows: u64,
    pub scrape_error_rows: u64,
    pub stage_rows: u64,
}
