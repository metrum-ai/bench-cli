// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Metrum AI Bench: the media fixtures shipped in `test-data/` must pass the
//! dummy server's `-strict-media` checks (and the negative fixture must not),
//! so the README quickstart never hands users media a real server rejects.
//! Skipped if `go` is missing.

mod common;

use common::{request_records, skip, spawn_dummy, summary_record};
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn run(bin: &str, args: &[&str], out: &Path) {
    let dir = out.parent().expect("out dir");
    let status = Command::new(bin)
        .current_dir(repo())
        .args(args)
        .args([
            "--api-key",
            "dummy",
            "--model",
            "dummy",
            "--concurrency",
            "1",
            "--data-log",
            out.to_str().unwrap(),
            "--debug-log",
            dir.join("debug.log").to_str().unwrap(),
            "--error-log",
            dir.join("error.log").to_str().unwrap(),
            "--log-level",
            "error",
        ])
        .status()
        .expect("run bench");
    assert!(status.success(), "{bin} failed");
}

/// The three LibriSpeech clips upload cleanly and every record carries WER
/// and CER from `test-data/asr/truth.jsonl`.
#[test]
fn shipped_asr_fixtures_pass_strict_media_and_score_wer() {
    let Some(dummy) = spawn_dummy(&[]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let dir = tempfile::tempdir().expect("tmpdir");
    let out = dir.path().join("asr.jsonl");
    run(
        env!("CARGO_BIN_EXE_metrum-ai-bench-cli-asr"),
        &[
            "--url",
            &dummy.url("/v1/audio/transcriptions"),
            "--scenario",
            "fixtures-asr",
            "--num-requests",
            "3",
            "--input",
            "test-data/asr/input.jsonl",
            "--ground-truth",
            "test-data/asr/truth.jsonl",
        ],
        &out,
    );
    let records = request_records(&out);
    assert_eq!(records.len(), 3);
    for record in &records {
        assert!(
            record["error"].is_null(),
            "shipped audio rejected: {}",
            record["error"]
        );
        let metrics = &record["modality_metrics"];
        assert!(metrics["wer"].as_f64().is_some(), "no wer: {metrics}");
        assert!(metrics["cer"].as_f64().is_some(), "no cer: {metrics}");
        assert!(
            metrics["audio_duration_s"]
                .as_f64()
                .is_some_and(|d| d > 1.0),
            "manifest duration not applied: {metrics}"
        );
    }
    assert!(summary_record(&out).is_some());
}

/// The negative fixture is rejected the way vLLM rejects it.
#[test]
fn negative_mp3_fixture_is_rejected_in_strict_media() {
    let Some(dummy) = spawn_dummy(&[]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let dir = tempfile::tempdir().expect("tmpdir");
    let input = dir.path().join("input.jsonl");
    let audio = repo().join("test-data/negative/header-only-invalid.mp3");
    std::fs::write(
        &input,
        format!(
            "{}\n",
            serde_json::json!({"id": "neg", "path": audio, "format": "mp3", "duration": 1.0})
        ),
    )
    .expect("write input");
    let out = dir.path().join("asr.jsonl");
    run(
        env!("CARGO_BIN_EXE_metrum-ai-bench-cli-asr"),
        &[
            "--url",
            &dummy.url("/v1/audio/transcriptions"),
            "--scenario",
            "fixtures-negative",
            "--num-requests",
            "1",
            "--input",
            input.to_str().unwrap(),
        ],
        &out,
    );
    let records = request_records(&out);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["error"]["kind"], "http_status");
    assert_eq!(records[0]["error"]["status"], 400);
}

/// The 512x512 VLM fixture passes strict media and is sent unchanged.
#[test]
fn shipped_vlm_fixture_passes_strict_media() {
    let Some(dummy) = spawn_dummy(&[]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let dir = tempfile::tempdir().expect("tmpdir");
    let out = dir.path().join("vlm.jsonl");
    run(
        env!("CARGO_BIN_EXE_metrum-ai-bench-cli-vlm"),
        &[
            "--url",
            &dummy.url("/v1/chat/completions"),
            "--scenario",
            "fixtures-vlm",
            "--num-requests",
            "1",
            "--max-tokens",
            "8",
            "--prompts",
            "test-data/vlm/prompts.jsonl",
        ],
        &out,
    );
    let records = request_records(&out);
    assert_eq!(records.len(), 1);
    assert!(records[0]["error"].is_null(), "{}", records[0]["error"]);
    let png = std::fs::metadata(repo().join("test-data/vlm/shapes-512.png"))
        .expect("stat png")
        .len() as f64;
    assert_eq!(records[0]["modality_metrics"]["image_bytes"], png);
}
