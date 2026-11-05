// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Historical fixtures captured from released CPU binaries with a local mock.

use metrum_ai_bench::error::RequestError;
use metrum_ai_bench::record::{Phase, RequestRecord};
use metrum_ai_bench::summary::RunSummary;
use metrum_ai_bench::{compare, count_points, record, telemetry::Row};
use serde_json::{json, Value};
use std::time::Duration;

// BC-SCHEMA-001: historical v3/v4 request and summary audit readers.
#[test]
fn released_request_summaries_remain_readable_by_data_point_audit() {
    for fixture in [
        include_str!("fixtures/v171-non-reasoning.json"),
        include_str!("fixtures/v171-released-request-summary.json"),
        include_str!("fixtures/v172-request-summary.json"),
    ] {
        let mut original: Value = serde_json::from_str(fixture).unwrap();
        for key in ["record", "summary"] {
            let row = original[key].as_object_mut().unwrap();
            let version = row["schema_version"].as_str().unwrap();
            assert!(record::accepts_audit_schema(version));
            row.insert("future_optional".into(), json!("ignored"));
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        std::fs::write(
            &path,
            format!("{}\n{}\n", original["record"], original["summary"]),
        )
        .unwrap();
        let result = count_points::count_bench(&path, 1.0).unwrap();
        assert_eq!(result["measured_requests"], 1);
        assert_eq!(
            original["record"]["completion_tokens"],
            original["summary"]["completion_tokens_total"]
        );
    }
    let old: Value =
        serde_json::from_str(include_str!("fixtures/v172-request-summary.json")).unwrap();
    assert!(
        old["record"]["ttft_first_any_s"].as_f64().unwrap()
            < old["record"]["ttft_s"].as_f64().unwrap()
    );
}

// BC-MATH-003: fixed historical v4 golden, zero window and incomplete usage.
#[test]
fn bc_math_003_undefined_usage_and_v4_legacy_golden() {
    let old: Value =
        serde_json::from_str(include_str!("fixtures/v172-request-summary.json")).unwrap();
    let row = &old["record"];
    let latency = Duration::from_secs_f64(row["latency_s"].as_f64().unwrap());
    let now = chrono::Utc::now();
    let record = RequestRecord::success(
        0,
        Phase::Measure,
        "fixture".into(),
        now,
        now,
        latency,
        Some(Duration::from_millis(1)),
        None,
        Vec::new(),
        row["prompt_tokens"].as_u64().unwrap(),
        row["completion_tokens"].as_u64().unwrap(),
        row["total_tokens"].as_u64().unwrap(),
    )
    .with_send_offset(Duration::ZERO);
    let summary =
        RunSummary::from_records(std::slice::from_ref(&record), latency.as_secs_f64(), false);
    let golden = old["summary"]["completion_tokens_per_second"]
        .as_f64()
        .unwrap();
    assert!((summary.legacy_success_completion_tokens_per_second.unwrap() - golden).abs() < 1e-9);
    let zero = RunSummary::from_records(std::slice::from_ref(&record), 0.0, false);
    assert!(zero.requests_per_second.is_none());
    assert!(zero.completion_tokens_per_second.is_none());
    assert!(zero.goodput.requests_per_second.is_none());
    assert_eq!(zero.throughput_bins_rps.n, 0);
    let mut missing = record.clone();
    missing.usage_missing = true;
    missing.tokenized_completion_tokens = Some(20);
    let gap = RunSummary::from_records(&[missing], 1.0, false);
    assert_eq!(gap.usage_missing_count, 1);
    assert!(gap.completion_tokens_per_second.is_none());
    assert_eq!(
        gap.legacy_success_completion_tokens_per_second,
        Some(20.0 / latency.as_secs_f64())
    );
    let mut truncated = record;
    truncated.ttft_visible_answer_s = None;
    truncated.first_reasoning_s = Some(0.1);
    truncated = truncated.with_chat_outcome(Some(RequestError::ReasoningTruncated), true);
    let strict = RunSummary::from_records(&[truncated], 1.0, false);
    assert_eq!(strict.successes, 0);
    assert_eq!(strict.answered_requests, 0);
    assert_eq!(strict.completion_tokens_per_second, Some(2.0));
    assert_eq!(strict.goodput.requests_per_second, Some(0.0));
}

// BC-SCHEMA-001: historical strategic CSV/JSON and telemetry reader compatibility.
#[test]
fn released_strategic_and_telemetry_fields_remain_readable() {
    for (strategic_fixture, csv_fixture, telemetry_fixture) in [
        (
            include_str!("fixtures/v171-released-strategic.json"),
            include_str!("fixtures/v171-released-strategic.csv"),
            include_str!("fixtures/v171-released-telemetry-request.json"),
        ),
        (
            include_str!("fixtures/v172-strategic.json"),
            include_str!("fixtures/v172-strategic.csv"),
            include_str!("fixtures/v172-telemetry-request.json"),
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("strategic.json");
        let mut strategic: Value = serde_json::from_str(strategic_fixture).unwrap();
        strategic["points"][0]["future_optional"] = json!("ignored");
        std::fs::write(&path, serde_json::to_vec(&strategic).unwrap()).unwrap();
        let run = compare::load_run(&path, "v1.7.2").unwrap();
        assert_eq!(run.points.len(), 1);
        assert_eq!(
            run.points[0].throughput,
            strategic["points"][0]["throughput"].as_f64()
        );
        assert_eq!(
            run.points[0].goodput,
            strategic["points"][0]["goodput"].as_f64()
        );
        let csv_path = dir.path().join("historical.csv");
        std::fs::write(&csv_path, csv_fixture).unwrap();
        let old_rows: Vec<metrum_ai_bench::strategic::BenchRecord> =
            csv::Reader::from_path(&csv_path)
                .unwrap()
                .deserialize()
                .map(Result::unwrap)
                .collect();
        assert_eq!(old_rows.len(), 1);
        assert!(
            old_rows[0].usage_missing,
            "legacy CSV has no usage coverage marker"
        );
        assert_eq!(old_rows[0].output_tokens, 2);
        let csv_run = compare::load_run(&csv_path, "v1.7.2 csv").unwrap();
        assert_eq!(csv_run.points.len(), 1);
        assert!(
            (csv_run.points[0].throughput.unwrap() - run.points[0].throughput.unwrap()).abs()
                < 1e-9
        );
        let telemetry: Row = serde_json::from_str(telemetry_fixture).unwrap();
        let Row::Request(request) = telemetry else {
            panic!("expected request row")
        };
        assert_eq!((request.input_tokens, request.output_tokens), (4, 2));
        assert_eq!(request.reasoning_tokens, Some(1));
        assert!(request.ttft_visible_answer_s.is_none());
        assert!(request.usage_missing.is_none());
    }
}
