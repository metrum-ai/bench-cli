// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use serde_json::{json, Value};

fn validator() -> jsonschema::Validator {
    let schema: Value =
        serde_json::from_str(include_str!("../schemas/kv-study-report.v1.json")).unwrap();
    jsonschema::validator_for(&schema).unwrap()
}

fn manifest() -> Value {
    json!({
        "schema_version": "kv-study-report.v1", "study_id": "offline-fixture",
        "reuse_kind": "exact_prefix",
        "runtime": {
            "model_revision": null, "tokenizer_revision": null, "chat_template_sha256": null,
            "engine": "mock", "engine_version": "1", "connector": null, "connector_version": null,
            "flags_sha256": null, "block_size_tokens": null, "kv_dtype": null, "kv_layout": null,
            "tensor_parallelism": null, "routing": null, "tiers": []
        },
        "workload": {
            "cache_policy": "reuse", "order": "preserve", "effective_order_sha256": null,
            "body_fingerprints_sha256": null, "history_mode": "recorded", "arrival_mode": "closed_loop",
            "concurrency": 1, "input_distribution": null, "output_distribution": null,
            "warmup_isolation": "unverified"
        },
        "datasets": [{
            "source_id": "synthetic-fixture", "resolved_revision": null, "config": null, "split": null,
            "files": [{"path": "fixture.jsonl", "sha256": "a".repeat(64), "bytes": 12}],
            "adapter_id": "fixture", "adapter_version": "1", "license_spdx": "Apache-2.0",
            "validation_manifest": null
        }],
        "phases": [{
            "id": "populate-1", "kind": "cold_populate", "cache_state": "unknown",
            "reset": {"scope": [], "acknowledged": false, "ready": false, "settled": false,
                "status": "unverified", "reason": "No reset evidence",
                "started_unix_ns": null, "acknowledged_unix_ns": null,
                "ready_unix_ns": null, "settled_unix_ns": null},
            "requests": 1, "errors": 0, "cached_token_known": 0, "cached_token_unknown": 1
        }],
        "limitations": ["Offline contract fixture only; no engine validation"]
    })
}

#[test]
fn version_and_required_provenance_are_strict() {
    let validator = validator();
    let base = manifest();
    assert!(validator.is_valid(&base));
    for field in ["runtime", "workload", "datasets", "phases", "limitations"] {
        let mut bad = base.clone();
        bad.as_object_mut().unwrap().remove(field);
        assert!(!validator.is_valid(&bad), "missing {field}");
    }
    let mut bad = base.clone();
    bad["schema_version"] = json!("kv-study-report.v2");
    assert!(!validator.is_valid(&bad));
    bad = base.clone();
    bad["datasets"][0]["files"][0]["sha256"] = json!("not-a-digest");
    assert!(!validator.is_valid(&bad));
    bad = base.clone();
    bad["datasets"][0]["files"][0]
        .as_object_mut()
        .unwrap()
        .remove("bytes");
    assert!(!validator.is_valid(&bad));
    let mut referenced = base.clone();
    referenced["datasets"][0]["validation_manifest"] = json!({
        "schema_version": "kv-dataset-manifest.v1", "sha256": "b".repeat(64)
    });
    assert!(validator.is_valid(&referenced));
    referenced["datasets"][0]["validation_manifest"]["sha256"] = json!("invalid");
    assert!(!validator.is_valid(&referenced));
    bad = base;
    bad["runtime"]["unversioned_extension"] = json!(true);
    assert!(!validator.is_valid(&bad));
}

#[test]
fn warmup_override_retains_both_body_bindings_without_claiming_a_pass() {
    let validator = validator();
    let mut value = manifest();
    value["workload"]["warmup_overlap"] = json!({
        "warmup_body_digests_sha256": "a".repeat(64),
        "measured_body_digests_sha256": "b".repeat(64),
        "verdict": "Unknown", "override_unverified": true
    });
    assert!(validator.is_valid(&value));
    value["workload"]["warmup_isolation"] = json!("token_prefix_disjoint");
    assert!(!validator.is_valid(&value));
    value["workload"]["warmup_overlap"]["verdict"] = json!("Pass");
    assert!(validator.is_valid(&value));
    value["workload"]["warmup_overlap"]
        .as_object_mut()
        .unwrap()
        .remove("measured_body_digests_sha256");
    assert!(!validator.is_valid(&value));
}

#[test]
fn cold_claim_requires_scoped_acknowledged_ready_settled_reset() {
    let validator = validator();
    let mut value = manifest();
    value["phases"][0]["cache_state"] = json!("verified_cold");
    assert!(!validator.is_valid(&value));
    value["phases"][0]["reset"] = json!({
        "scope": ["resident", "external"], "acknowledged": true, "ready": true,
        "settled": true, "status": "verified", "reason": null,
        "started_unix_ns": 10, "acknowledged_unix_ns": 20,
        "ready_unix_ns": 30, "settled_unix_ns": 40
    });
    assert!(validator.is_valid(&value));
    for field in ["acknowledged", "ready", "settled"] {
        let mut bad = value.clone();
        bad["phases"][0]["reset"][field] = json!(false);
        assert!(!validator.is_valid(&bad));
    }
    for field in [
        "started_unix_ns",
        "acknowledged_unix_ns",
        "ready_unix_ns",
        "settled_unix_ns",
    ] {
        let mut bad = value.clone();
        bad["phases"][0]["reset"][field] = Value::Null;
        assert!(!validator.is_valid(&bad));
    }
    value["phases"][0]["reset"]["scope"] = json!([]);
    assert!(!validator.is_valid(&value));
}

#[tokio::test]
async fn offline_mock_unknown_usage_remains_unknown_in_contract() {
    use axum::{routing::post, Json, Router};
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/chat/completions",
        post(|| async { Json(json!({"usage": {"prompt_tokens": 16, "completion_tokens": 1}})) }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let response: Value = reqwest::Client::new()
        .post(format!("http://{address}/chat/completions"))
        .json(&json!({"messages": [{"role": "user", "content": "synthetic"}]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(response["usage"]["prompt_tokens_details"]["cached_tokens"].is_null());
    let value = manifest();
    assert!(validator().is_valid(&value));
    assert_eq!(value["phases"][0]["cached_token_unknown"], 1);
    assert_eq!(value["phases"][0]["cache_state"], "unknown");
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}
