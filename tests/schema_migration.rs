// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Version-aware wrapper contract fixture; downstream owner signoff is separate.
use chrono::Utc;
use metrum_ai_bench::error::RequestError;
use metrum_ai_bench::measurement::{ResolvedTtft, TtftSource};
use metrum_ai_bench::record::{
    Phase, RequestRecord, SCHEMA_VERSION_REQUEST, SCHEMA_VERSION_SUMMARY,
};
use metrum_ai_bench::summary::RunSummary;
use serde_json::{json, Value};
use std::time::Duration;

/// Frozen v1.7.2 count-points summary selection predicate. It explicitly
/// rejects unknown versions, including v5; it must not silently relabel them.
fn v172_reader(rows: &[Value]) -> Result<&Value, &'static str> {
    rows.iter()
        .rev()
        .find(|row| {
            row["schema_version"]
                .as_str()
                .is_some_and(|v| v.ends_with(".summary.v3") || v.ends_with(".summary.v4"))
        })
        .ok_or("no supported summary record")
}

#[derive(Debug, PartialEq)]
struct WrapperView {
    first_any: Option<f64>,
    visible: Option<f64>,
    answered: bool,
    outcome: String,
}

/// Independent version-aware consumer fixture, not the external wrapper.
fn wrapper_view(row: &Value) -> Result<WrapperView, &'static str> {
    match row["schema_version"].as_str() {
        Some("metrum-ai-bench-cli.request.v4") => Ok(WrapperView {
            first_any: row["ttft_first_any_s"].as_f64(),
            visible: row["ttft_s"].as_f64(),
            answered: row["error"].is_null() && row["ttft_s"].is_number(),
            outcome: row["error"]["kind"].as_str().unwrap_or("answer").into(),
        }),
        Some("metrum-ai-bench-cli.request.v5") => Ok(WrapperView {
            first_any: if row["ttft_source"] == "stream" {
                row["ttft_s"].as_f64()
            } else {
                None
            },
            visible: row["ttft_visible_answer_s"].as_f64(),
            answered: row["answer_completed"]
                .as_bool()
                .ok_or("answer state missing")?,
            outcome: row["outcome"].as_str().ok_or("outcome missing")?.into(),
        }),
        _ => Err("unsupported request schema"),
    }
}

// BC-SCHEMA-002: schema bumps, timing provenance and a version-aware wrapper
// contract; this fixture does not replace downstream integration signoff.
#[test]
fn bc_schema_002_versioned_output_and_wrapper_migration() {
    let old: Value =
        serde_json::from_str(include_str!("fixtures/v172-request-summary.json")).unwrap();
    let old_view = wrapper_view(&old["record"]).unwrap();
    assert!(old_view.first_any.unwrap() < old_view.visible.unwrap());
    assert!(old_view.answered);
    assert_eq!(
        v172_reader(&[old["summary"].clone()]).unwrap()["schema_version"],
        "metrum-ai-bench-cli.summary.v4"
    );
    let now = Utc::now();
    let mut row = RequestRecord::success(
        0,
        Phase::Measure,
        "fixture".into(),
        now,
        now,
        Duration::from_secs(1),
        None,
        Some(Duration::from_millis(100)),
        vec![],
        4,
        8,
        12,
    )
    .with_reasoning_tokens(Some(8))
    .with_resolved_ttft(ResolvedTtft {
        ttft_s: Some(0.1),
        source: Some(TtftSource::Stream),
    })
    .with_chat_outcome(Some(RequestError::ReasoningTruncated), false);
    row.usage_missing = false;
    let request = serde_json::to_value(&row).unwrap();
    let summary = serde_json::to_value(RunSummary::from_records(
        std::slice::from_ref(&row),
        1.0,
        false,
    ))
    .unwrap();
    assert_eq!(request["schema_version"], SCHEMA_VERSION_REQUEST);
    assert_eq!(summary["schema_version"], SCHEMA_VERSION_SUMMARY);
    assert_eq!(
        v172_reader(std::slice::from_ref(&summary)).unwrap_err(),
        "no supported summary record"
    );
    assert_eq!(
        wrapper_view(&request).unwrap(),
        WrapperView {
            first_any: Some(0.1),
            visible: None,
            answered: false,
            outcome: "reasoning_truncated".into()
        }
    );
    assert_eq!(summary["ttft_coverage"]["reasoning_truncated"], 1);
    assert_eq!(summary["answered_requests"], 0);
    assert_eq!(summary["completion_tokens_per_second"], 8.0);
    assert_eq!(summary["goodput"]["count"], 0);
    let telemetry = metrum_ai_bench::telemetry::request_row("fixture", &row, 0, 1.0);
    assert_eq!(
        metrum_ai_bench::telemetry::TELEMETRY_SCHEMA_VERSION,
        "metrum-ai-bench-cli.telemetry.v3"
    );
    assert_eq!(telemetry.ttft_s, Some(0.1));
    assert_eq!(telemetry.ttft_visible_answer_s, None);
    assert_eq!(telemetry.answer_completed, Some(false));
    assert_eq!(telemetry.outcome.as_deref(), Some("reasoning_truncated"));
    let mut unknown = request;
    unknown["schema_version"] = json!("metrum-ai-bench-cli.request.v99");
    assert_eq!(
        wrapper_view(&unknown).unwrap_err(),
        "unsupported request schema"
    );
}
