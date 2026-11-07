// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
use metrum_ai_bench::{
    cached_tokens::CachedTokenEvidence, kv_study_export::*, strategic::BenchRecord,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
fn manifest() -> Value {
    let mut runtime = json!({});
    for key in [
        "model_revision",
        "tokenizer_revision",
        "chat_template_sha256",
        "engine",
        "engine_version",
        "connector",
        "connector_version",
        "flags_sha256",
        "block_size_tokens",
        "kv_dtype",
        "kv_layout",
        "tensor_parallelism",
        "routing",
    ] {
        runtime[key] = Value::Null;
    }
    runtime["tiers"] = json!([]);
    json!({"schema_version":"kv-study-report.v1","study_id":"fixture","reuse_kind":"exact_prefix","runtime":runtime,"datasets":[],"limitations":["synthetic fixture, not deployment validation"],"workload":{"cache_policy":"reuse","order":"preserve","effective_order_sha256":null,"body_fingerprints_sha256":null,"history_mode":"recorded","arrival_mode":"closed_loop","concurrency":1,"input_distribution":null,"output_distribution":null,"warmup_isolation":"unverified"},"phases":[{"id":"=cmd<script>alert(1)</script>","kind":"cold_populate","cache_state":"unknown","reset":{"scope":[],"acknowledged":false,"ready":false,"settled":false,"status":"not_requested","reason":null,"started_unix_ns":null,"acknowledged_unix_ns":null,"ready_unix_ns":null,"settled_unix_ns":null},"requests":999,"errors":999,"cached_token_known":999,"cached_token_unknown":999}]})
}
fn row(success: bool, cached: Option<u64>, warmup: bool) -> BenchRecord {
    BenchRecord {
        success,
        warmup,
        ttft_s: Some(if success { 0.2 } else { 99.0 }),
        ttft_visible_answer_s: Some(0.3),
        cache_evidence: cached.map(|n| {
            CachedTokenEvidence::from_usage(
                &json!({"prompt_tokens":10,"prompt_tokens_details":{"cached_tokens":n}}),
            )
        }),
        ..Default::default()
    }
}
#[test]
fn all_views_bind_actual_counts_exclude_warmup_errors_and_preserve_null_provenance() {
    let seed = StudySeed::parse(&serde_json::to_vec(&manifest()).unwrap()).unwrap();
    let rows = vec![
        row(true, Some(0), false),
        row(true, None, false),
        row(false, Some(8), false),
        row(true, Some(10), true),
    ];
    let export = StudyExport::bind(
        seed,
        &BTreeMap::from([(
            "=cmd<script>alert(1)</script>".into(),
            PhaseRecords::Strategic(&rows),
        )]),
    )
    .unwrap();
    let json: Value = serde_json::from_slice(&export.json().unwrap()).unwrap();
    let schema: Value =
        serde_json::from_str(include_str!("../schemas/kv-study-report.v1.json")).unwrap();
    assert!(jsonschema::validator_for(&schema)
        .unwrap()
        .is_valid(&json["report"]));
    let phase = &json["phases"][0];
    assert_eq!(phase["requests"], 3);
    assert_eq!(phase["errors"], 1);
    assert_eq!(phase["cached_token_known"], 1);
    assert_eq!(phase["cached_token_unknown"], 2);
    assert_eq!(phase["cache_state"], "unknown");
    assert_eq!(phase["small_n_warning"], true);
    assert_eq!(phase["cohorts"]["warmup_excluded"], 1);
    assert_eq!(phase["cohorts"]["errors_excluded_from_timing"], 1);
    assert_eq!(json["report"]["phases"][0]["requests"], 3);
    assert_eq!(json["report"]["runtime"]["block_size_tokens"], Value::Null);
    let bytes = export.csv().unwrap();
    let mut csv = csv::Reader::from_reader(bytes.as_slice());
    let headers = csv.headers().unwrap().clone();
    let row = csv.records().next().unwrap().unwrap();
    for (name, value) in [
        ("requests", "3"),
        ("errors", "1"),
        ("cached_token_known", "1"),
        ("cached_token_unknown", "2"),
        ("small_n_warning", "true"),
        ("ttft_unit", "seconds"),
    ] {
        assert_eq!(
            row.get(headers.iter().position(|s| s == name).unwrap()),
            Some(value)
        );
    }
    assert!(
        row.get(2).unwrap().starts_with("phase: =cmd"),
        "spreadsheet formula must not start a free-text cell"
    );
    let cohort: Value = serde_json::from_str(
        row.get(headers.iter().position(|s| s == "cohorts_json").unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(cohort, phase["cohorts"]);
    let html = export.html().unwrap();
    assert!(!html.contains("<script>"));
    assert!(html.contains("&lt;script&gt;"));
    assert!(html.contains("Small-n"));
    assert_eq!(json["engineering_non_certifying"], true);
    assert!(!json["publication_blockers"].as_array().unwrap().is_empty());
}
#[test]
fn no_runtime_binding_cannot_promote_physical_state_or_observed_tiers() {
    let mut seed = manifest();
    seed["phases"][0]["cache_state"] = json!("observed_resident");
    assert!(StudySeed::parse(&serde_json::to_vec(&seed).unwrap()).is_err());
    let mut seed = manifest();
    seed["runtime"]["tiers"] = json!([{"id":"tier","capacity_bytes":null,"evidence":"observed"}]);
    assert!(StudySeed::parse(&serde_json::to_vec(&seed).unwrap()).is_err());
    let mut seed = manifest();
    let duplicate = seed["phases"][0].clone();
    seed["phases"].as_array_mut().unwrap().push(duplicate);
    assert!(StudySeed::parse(&serde_json::to_vec(&seed).unwrap()).is_err());
    let seed = StudySeed::parse(&serde_json::to_vec(&manifest()).unwrap()).unwrap();
    assert!(StudyExport::bind(seed, &BTreeMap::new()).is_err());
}
