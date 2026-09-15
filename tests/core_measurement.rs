// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use chrono::Utc;
use metrumbench::error::RequestError;
use metrumbench::record::{Phase, RequestRecord};
use metrumbench::summary::RunSummary;
use std::time::Duration;

fn record(seq: u64, phase: Phase, endpoint: &str, latency_ms: u64, ttft_ms: u64) -> RequestRecord {
    RequestRecord::success(
        seq,
        phase,
        endpoint.into(),
        Utc::now(),
        Duration::from_millis(latency_ms),
        Some(Duration::from_millis(ttft_ms)),
        None,
        vec![Duration::from_millis(20); 19],
        18,
        20,
        38,
    )
    .with_schedule(Duration::from_millis(seq * 125), Duration::from_millis(0))
}

#[test]
fn summary_matches_hand_computed_reference_16_requests() {
    let records: Vec<_> = (0..16)
        .map(|seq| record(seq, Phase::Measure, "dummy", 500, 120))
        .collect();
    let summary = RunSummary::from_records(&records, 2.019_56, false);
    assert_eq!(summary.latency_s.n, 16);
    assert_eq!(summary.ttft_s.n, 16);
    assert_eq!(summary.itl_s.n, 16 * 19);
    assert!((summary.ttft_s.avg.unwrap() - 0.120).abs() < 1e-12);
    assert!((summary.latency_s.avg.unwrap() - 0.500).abs() < 1e-12);
    assert!((summary.itl_s.avg.unwrap() - 0.020).abs() < 1e-12);
    assert!((summary.tpot_s.avg.unwrap() - 0.020).abs() < 1e-12);
    assert!((summary.completion_tokens_per_second - 158.450_355_523).abs() < 1e-6);
    assert!((summary.requests_per_second - 7.922_517_776).abs() < 1e-6);
}

#[test]
fn summary_window_excludes_warmup_and_drain() {
    let records = [
        record(0, Phase::Warmup, "dummy", 900, 400),
        record(1, Phase::Measure, "dummy", 100, 20),
        record(2, Phase::Drain, "dummy", 800, 300),
    ];
    let summary = RunSummary::from_records(&records, 0.5, false);
    assert_eq!(summary.attempted, 1);
    assert_eq!(summary.latency_s.n, 1);
    assert_eq!(summary.requests_per_second, 2.0);
}

#[test]
fn undefined_statistics_serialize_as_null_not_zero() {
    let summary = RunSummary::from_records(&[], 1.0, false);
    let value = serde_json::to_value(summary).unwrap();
    assert_eq!(value["latency_s"]["n"], 0);
    assert!(value["latency_s"]["avg"].is_null());
}

#[test]
fn no_output_and_http_status_are_typed() {
    let records = [
        RequestRecord::failed(
            0,
            Phase::Measure,
            "dummy".into(),
            Utc::now(),
            Duration::from_millis(1),
            RequestError::NoOutputToken,
        ),
        RequestRecord::failed(
            1,
            Phase::Measure,
            "dummy".into(),
            Utc::now(),
            Duration::from_millis(1),
            RequestError::HttpStatus { status: 503 },
        ),
    ];
    let summary = RunSummary::from_records(&records, 1.0, false);
    assert_eq!(summary.errors, 2);
    assert_eq!(summary.errors_by_type["no_output_token"], 1);
    assert_eq!(summary.errors_by_type["http_status"], 1);
}

#[test]
fn coordinated_omission_includes_queue_delay() {
    let fast = record(0, Phase::Measure, "dummy", 100, 20);
    let slow = record(1, Phase::Measure, "dummy", 100, 20)
        .with_schedule(Duration::from_millis(100), Duration::from_millis(900));
    let summary = RunSummary::from_records(&[fast, slow], 1.0, false);
    assert_eq!(summary.latency_s.max, Some(0.1));
    assert_eq!(summary.coordinated_omission_latency_s.max, Some(1.0));
}

#[test]
fn all_warmup_produces_empty_measurement_not_fallback() {
    let summary =
        RunSummary::from_records(&[record(0, Phase::Warmup, "dummy", 100, 20)], 1.0, false);
    assert_eq!(summary.attempted, 0);
    assert_eq!(summary.latency_s.n, 0);
}
