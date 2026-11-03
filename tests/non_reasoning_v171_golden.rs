// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Fixed-input compatibility against the v1.7.1 release, excluding only
//! version identifiers and fields added by v1.7.2. The fixture was generated
//! from v1.7.1 commit 3e80169 using the identical fixed inputs below; the
//! environment is deliberately empty to avoid machine-specific evidence.
use chrono::{TimeZone, Utc};
use metrum_ai_bench::{
    record::{Phase, RequestRecord},
    summary::RunSummary,
};
use serde_json::{json, Value};
use std::time::Duration;

fn normalize(value: &mut Value) {
    let root = value.as_object_mut().unwrap();
    let record = root.get_mut("record").unwrap().as_object_mut().unwrap();
    record.remove("schema_version");
    record.remove("ttft_first_any_s");
    record.remove("finish_reason");
    let summary = root.get_mut("summary").unwrap().as_object_mut().unwrap();
    for key in [
        "schema_version",
        "ttft_coverage",
        "observed_reasoning_tokens",
        "observed_reasoning_tokens_total",
        "observed_visible_completion_tokens",
        "observed_visible_completion_tokens_total",
        "fallback_ttft_s",
    ] {
        summary.remove(key);
    }
}

#[test]
fn non_reasoning_schema_and_values_match_v171_after_normalization() {
    let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
    let record = RequestRecord::success(
        0,
        Phase::Measure,
        "example".into(),
        now,
        now,
        Duration::from_millis(1000),
        Some(Duration::from_millis(200)),
        None,
        vec![Duration::from_millis(100)],
        10,
        20,
        30,
    )
    .with_first_byte(Duration::from_millis(50));
    let mut summary = RunSummary::from_records(std::slice::from_ref(&record), 1.0, false);
    // Keep the golden independent of machine identity and software environment.
    summary.environment = json!({});
    let mut actual = json!({"record": record, "summary": summary});
    let mut baseline: Value =
        serde_json::from_str(include_str!("fixtures/v171-non-reasoning.json")).unwrap();
    // This field existed in v1.7.1 but was omitted for non-reasoning runs.
    // Only its newly emitted value may be ignored for this specific fixture.
    assert!(baseline["summary"].get("ttft_first_any_s").is_none());
    actual["summary"]
        .as_object_mut()
        .unwrap()
        .remove("ttft_first_any_s")
        .expect("v1.7.2 always emits the first-any distribution");
    normalize(&mut actual);
    normalize(&mut baseline);
    assert_eq!(
        serde_json::to_vec(&actual).unwrap(),
        serde_json::to_vec(&baseline).unwrap()
    );
}
