// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use metrum_ai_bench::public_dataset::sha256;
use std::{
    path::Path,
    process::{Command, Output},
};
fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-prompts"))
        .current_dir(dir)
        .env_remove("HF_TOKEN")
        .arg("--quiet")
        .args(args)
        .output()
        .unwrap()
}
fn fixture(dir: &Path) {
    let bytes=b"{\"id\":\"private-row\",\"doc_id\":\"doc\",\"context\":\"shared content\",\"question\":\"question\",\"answer\":\"GOLD-NEVER-COPY\"}\n";
    std::fs::write(dir.join("source.jsonl"), bytes).unwrap();
    let job = serde_json::json!({
        "schema_version":"public-dataset-job.v1","origin":{"source_id":"example/synthetic","resolved_revision":"a".repeat(40),"config":"test","split":"test","files":[{"path":"source.jsonl","sha256":sha256(bytes),"bytes":bytes.len()}]},
        "rights":{"decision":"allowed","license":"Apache-2.0","license_text_sha256":"b".repeat(64),"component_terms":{},"gated":false,"terms_review":null,"redistribution_allowed":false,"privacy_handling":"synthetic only","unresolved":[]},
        "mapping":{"adapter_id":"synthetic","adapter_version":"1","expected_config":"test","expected_split":"test","row_id_field":"id","document_id_field":"doc_id","document_field":"context","questions_field":"question","question_array":false,"output_budget":8},
        "formats":{"source.jsonl":{"format":"jsonl"}},"local_files":{"source.jsonl":dir.join("source.jsonl")},
        "limits":{"max_file_bytes":16384,"max_total_bytes":16384,"max_rows":8,"max_targets":8,"max_body_bytes":16384,"max_pairs":64},"count":null,"constraints":{"content_unique":false,"max_pair_lcp":null},"block_size_tokens":2,"rendering":null
    });
    std::fs::write(dir.join("job.json"), serde_json::to_vec(&job).unwrap()).unwrap();
}
#[test]
fn catalog_listing_is_metadata_only_and_schema_valid() {
    let dir = tempfile::tempdir().unwrap();
    let output = run(dir.path(), &["catalog"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["automatic_access_granted"], false);
    assert_eq!(value["source_checks_run"], false);
    assert_eq!(value["entries"].as_array().unwrap().len(), 8);
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../schemas/optional-public-catalog.v1.json")).unwrap();
    assert!(jsonschema::validator_for(&schema).unwrap().is_valid(&value));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
#[test]
fn dataset_consolidation_keeps_payload_separate_unknown_and_no_clobber() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let args = [
        "dataset",
        "--job",
        "job.json",
        "--output",
        "private.jsonl",
        "--report",
        "profile.json",
        "--consolidation-report",
        "metadata.json",
        "--offline",
    ];
    let output = run(dir.path(), &args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload = std::fs::read(dir.path().join("private.jsonl")).unwrap();
    assert!(!String::from_utf8_lossy(&payload).contains("GOLD-NEVER-COPY"));
    let metadata = std::fs::read(dir.path().join("metadata.json")).unwrap();
    assert!(!String::from_utf8_lossy(&metadata).contains("private-row"));
    let report: serde_json::Value = serde_json::from_slice(&metadata).unwrap();
    assert_eq!(report["checks"]["rendered_sample"]["status"], "unknown");
    assert_eq!(
        report["checks"]["declared_component_coverage"]["status"],
        "unknown"
    );
    assert_eq!(
        report["checks"]["physical_cache_observations"]["status"],
        "unknown"
    );
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../schemas/source-validation-consolidation.v1.json"
    ))
    .unwrap();
    assert!(jsonschema::validator_for(&schema)
        .unwrap()
        .is_valid(&report));
    assert!(!run(dir.path(), &args).status.success());
    assert_eq!(
        std::fs::read(dir.path().join("private.jsonl")).unwrap(),
        payload
    );
    assert_eq!(
        std::fs::read(dir.path().join("metadata.json")).unwrap(),
        metadata
    );
}
#[test]
fn alias_outputs_and_unreviewed_catalog_fail_before_publishing_payload() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    std::fs::create_dir(dir.path().join("nested")).unwrap();
    let output = run(
        dir.path(),
        &[
            "dataset",
            "--job",
            "job.json",
            "--output",
            "private.jsonl",
            "--report",
            "profile.json",
            "--consolidation-report",
            "nested/../private.jsonl",
            "--offline",
        ],
    );
    assert!(!output.status.success());
    assert!(!dir.path().join("private.jsonl").exists());
    assert!(!dir.path().join("profile.json").exists());
    let output = run(
        dir.path(),
        &[
            "dataset",
            "--job",
            "job.json",
            "--output",
            "private.jsonl",
            "--report",
            "profile.json",
            "--catalog-key",
            "loogle-longdep-qa",
            "--offline",
        ],
    );
    assert!(!output.status.success());
    assert!(!dir.path().join("private.jsonl").exists());
}
