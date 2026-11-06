// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use serde_json::{json, Value};

fn schema() -> Value {
    serde_json::from_str(include_str!("../schemas/kv-dataset-manifest.v1.json")).unwrap()
}

fn fixture() -> Value {
    json!({
        "schema_version": "kv-dataset-manifest.v1",
        "origin": {"source_id": "example/fixture", "resolved_revision": "a".repeat(40), "config": "default", "split": "test", "files": [{"path": "fixture.jsonl", "sha256": "b".repeat(64), "bytes": 3}]},
        "rights": {"decision": "review_required", "license": null, "license_text_sha256": null, "component_terms": {}, "gated": false, "terms_review": null, "redistribution_allowed": false, "privacy_handling": "metadata only", "unresolved": ["license unknown"]},
        "adapter_id": "fixture", "adapter_version": "1", "field_mapping": {"text": "prompt"}, "transformations": [],
        "tokenizer_revision": null, "chat_template_sha256": null, "generator_version": null,
        "corpus_rows": {"status": "unknown"}, "selected_source_rows": 0, "target_count": 0,
        "selected_ids_sha256": "c".repeat(64), "body_order_sha256": "d".repeat(64),
        "checks": {"rendered_overlap": {"status": "unknown"}}, "exclusions": [], "limitations": ["No payload accessed"]
    })
}

#[test]
fn unknown_rights_are_reportable_not_implicitly_allowed() {
    let validator = jsonschema::validator_for(&schema()).unwrap();
    let base = fixture();
    assert!(validator.is_valid(&base));
    let manifest: metrum_ai_bench::public_dataset::ValidationManifest =
        serde_json::from_value(base).unwrap();
    assert!(manifest.rights.authorize().is_err());
    assert!(manifest.fingerprint().is_ok());
    let mut bad = fixture();
    bad["corpus_rows"] = json!({"status": "unknown", "value": 0});
    assert!(!validator.is_valid(&bad));
    for field in ["rights", "adapter_id", "origin", "checks", "limitations"] {
        let mut bad = fixture();
        bad.as_object_mut().unwrap().remove(field);
        assert!(!validator.is_valid(&bad), "missing {field}");
    }
    let mut bad = fixture();
    bad["schema_version"] = json!(1);
    assert!(!validator.is_valid(&bad));
    let mut bad = fixture();
    bad["origin"]["files"][0]["sha256"] = json!("invalid");
    assert!(!validator.is_valid(&bad));
}

#[test]
fn report_envelope_names_and_reference_version_are_exact() {
    let manifest: metrum_ai_bench::public_dataset::ValidationManifest =
        serde_json::from_value(fixture()).unwrap();
    let envelope = serde_json::to_value(manifest.envelope().unwrap()).unwrap();
    let mut envelope_schema = schema();
    envelope_schema["$ref"] = json!("#/$defs/envelope");
    envelope_schema.as_object_mut().unwrap().remove("required");
    envelope_schema
        .as_object_mut()
        .unwrap()
        .remove("properties");
    envelope_schema
        .as_object_mut()
        .unwrap()
        .remove("additionalProperties");
    let validator = jsonschema::validator_for(&envelope_schema).unwrap();
    assert!(validator.is_valid(&envelope));
    assert_eq!(
        envelope["validation_manifest"]["schema_version"],
        "kv-dataset-manifest.v1"
    );
    let mut bad = envelope.clone();
    bad["unexpected"] = json!(true);
    assert!(!validator.is_valid(&bad));
    let mut bad = envelope;
    bad["validation_manifest"]["sha256"] = json!("not-a-digest");
    assert!(!validator.is_valid(&bad));
}
