// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Strategic-run telemetry: tagged NDJSON rows, shared monotonic epoch, and
//! Prometheus exposition scraping.

mod config;
mod epoch;
mod parser;
mod row;
mod scraper;
mod writer;

pub use config::{
    TelemetryConfig, TelemetrySource, UnitScale, DEFAULT_INTERVAL_MS, DEFAULT_MAX_BODY_BYTES,
    DEFAULT_REQUIRE_FAILURES, DEFAULT_TIMEOUT_MS, MIN_INTERVAL_MS,
};
pub use epoch::RunEpoch;
pub use parser::{engine_include_patterns, parse_exposition, ParseResult, ParsedSample};
pub use row::{
    MetricType, PhaseKind, RequestRow, Row, RunRow, ScrapeErrorRow, StageRow, SummaryRow,
    TelemetryRow, TelemetrySourceStamp, TELEMETRY_SCHEMA_VERSION,
};
pub use scraper::{
    build_telemetry_client, default_require_failures, new_last_seen, probe_sources, spawn_scrapers,
    LastSeenMap, ProbeResult,
};
pub use writer::{NdjsonWriter, WriterStats, TELEMETRY_CHANNEL_CAPACITY};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn writer_emits_tagged_rows_and_flushes() {
        let file = NamedTempFile::new().expect("temp");
        let path = file.path().to_path_buf();
        let (writer, handle) = NdjsonWriter::spawn(path.clone()).expect("spawn");
        let epoch = RunEpoch::new();
        writer
            .send_priority(Row::Run(RunRow {
                run_id: "run-1".into(),
                t0_wall: epoch.t0_wall_iso(),
                tool_version: "0.0.0".into(),
                schema_version: TELEMETRY_SCHEMA_VERSION.into(),
                sut: None,
                config: json!({}),
                telemetry_sources: vec![],
            }))
            .await
            .expect("send run");
        writer
            .send_priority(Row::Request(RequestRow {
                run_id: "run-1".into(),
                seq: 1,
                stage: 1.0,
                warmup: false,
                t_sched_ns: 0,
                t_sent_ns: 1_000,
                t_first_ns: Some(2_000),
                t_done_ns: 3_000,
                success: true,
                input_tokens: 10,
                output_tokens: 20,
                latency_s: 0.003,
                queue_delay_s: 0.0,
                service_latency_s: 0.003,
                ttft_s: Some(0.001),
                error: None,
                telemetry_at_done: None,
            }))
            .await
            .expect("send request");
        writer
            .send_priority(Row::Summary(SummaryRow {
                run_id: "run-1".into(),
                partial: false,
                dropped_telemetry_rows: 0,
                request_rows: 1,
                telemetry_rows: 0,
                scrape_error_rows: 0,
                stage_rows: 0,
            }))
            .await
            .expect("send summary");
        drop(writer);
        let stats = handle.shutdown().await.expect("shutdown");
        assert_eq!(stats.dropped_telemetry_rows, 0);
        assert_eq!(stats.written_rows, 3);
        let text = std::fs::read_to_string(&path).expect("read");
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        let first: serde_json::Value = serde_json::from_str(lines[0]).expect("json");
        assert_eq!(first["kind"], "run");
        assert_eq!(first["run_id"], "run-1");
        let second: serde_json::Value = serde_json::from_str(lines[1]).expect("json");
        assert_eq!(second["kind"], "request");
        assert_eq!(second["t_sent_ns"], 1000);
        let third: serde_json::Value = serde_json::from_str(lines[2]).expect("json");
        assert_eq!(third["kind"], "summary");
        assert_eq!(third["partial"], false);
    }

    #[tokio::test]
    async fn telemetry_rows_drop_under_backpressure() {
        let file = NamedTempFile::new().expect("temp");
        let (writer, handle) = NdjsonWriter::spawn(file.path().to_path_buf()).expect("spawn");
        let mut drops = 0u64;
        for i in 0..20_000 {
            let dropped = writer.try_send_telemetry(Row::Telemetry(TelemetryRow {
                run_id: "run-1".into(),
                t_ns: i,
                src: "dcgm".into(),
                metric: "DCGM_FI_DEV_POWER_USAGE".into(),
                labels: BTreeMap::new(),
                value: 100.0,
                unit: "W".into(),
                mtype: MetricType::Gauge,
                scrape_ms: 1.0,
                raw: None,
            }));
            if dropped {
                drops += 1;
            }
            if drops > 0 && i > TELEMETRY_CHANNEL_CAPACITY as u64 + 100 {
                break;
            }
        }
        assert!(drops > 0, "expected telemetry drops under backpressure");
        drop(writer);
        let _ = handle.shutdown().await;
    }

    #[test]
    fn epoch_offsets_are_monotonic() {
        let epoch = RunEpoch::new();
        let a = epoch.elapsed_ns();
        std::thread::sleep(std::time::Duration::from_millis(1));
        let b = epoch.elapsed_ns();
        assert!(b > a);
        let iso = epoch.t0_wall_iso();
        assert!(iso.ends_with('Z') || iso.contains('+'));
    }
}
