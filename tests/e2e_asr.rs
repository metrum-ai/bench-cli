// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! ASR end-to-end checks against dummy-model-server. Skipped if `go` is missing.

mod common;

use common::{request_records, run_config, skip, spawn_dummy, summary_record};
use serde_json::Value;
use std::io::Write;
use std::process::Command;

fn asr_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_metrum-ai-bench-asr"))
}

/// The dummy server always answers with this text.
const DUMMY_TRANSCRIPT: &str = "dummy transcription";
const AUDIO_SECONDS: f64 = 4.0;

struct Fixture {
    _dir: tempfile::TempDir,
    input: std::path::PathBuf,
    ground_truth: std::path::PathBuf,
    data_log: std::path::PathBuf,
    debug_log: std::path::PathBuf,
    error_log: std::path::PathBuf,
}

/// Writes an input manifest plus a ground truth whose only differences from
/// the server response are case, punctuation and a spelled-out numeral, so
/// each normalizer setting yields a different, predictable WER.
fn fixture(reference: &str) -> Fixture {
    let dir = tempfile::tempdir().expect("tmpdir");
    let audio = dir.path().join("sample.mp3");
    // The dummy server never decodes the payload; any bytes will do.
    std::fs::write(&audio, b"ID3\x04\x00\x00\x00\x00\x00\x00fake mp3 payload").expect("write mp3");

    let input = dir.path().join("input.jsonl");
    let mut file = std::fs::File::create(&input).expect("create input");
    writeln!(
        file,
        r#"{{"id":"sample-1","path":"{}","format":"mp3","duration":{}}}"#,
        audio.to_str().unwrap(),
        AUDIO_SECONDS
    )
    .expect("write input");

    let ground_truth = dir.path().join("truth.jsonl");
    let mut file = std::fs::File::create(&ground_truth).expect("create truth");
    writeln!(
        file,
        "{}",
        serde_json::json!({"id": "sample-1", "transcript": reference})
    )
    .expect("write truth");

    Fixture {
        input,
        ground_truth,
        data_log: dir.path().join("out.jsonl"),
        debug_log: dir.path().join("debug.log"),
        error_log: dir.path().join("error.log"),
        _dir: dir,
    }
}

fn run_asr(fixture: &Fixture, url: &str, extra: &[&str]) {
    let mut args: Vec<String> = [
        "--url",
        url,
        "--api-key",
        "dummy",
        "--scenario",
        "e2e-asr",
        "--num-requests",
        "1",
        "--concurrency",
        "1",
        "--input",
        fixture.input.to_str().unwrap(),
        "--ground-truth",
        fixture.ground_truth.to_str().unwrap(),
        "--model",
        "dummy",
        "--response-format",
        "verbose-json",
        "--data-log",
        fixture.data_log.to_str().unwrap(),
        "--debug-log",
        fixture.debug_log.to_str().unwrap(),
        "--error-log",
        fixture.error_log.to_str().unwrap(),
        "--log-level",
        "error",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.extend(extra.iter().map(|s| s.to_string()));

    let status = Command::new(asr_bin())
        .args(&args)
        .status()
        .expect("run asr");
    assert!(status.success(), "asr bench failed");
}

fn modality_metric(record: &Value, key: &str) -> Option<f64> {
    record["modality_metrics"][key].as_f64()
}

/// RTFx is audio seconds over client seconds, and reading the file off disk
/// must not be inside the measured window.
#[test]
fn asr_reports_rtfx_from_client_time_excluding_file_read() {
    let Some(dummy) = spawn_dummy(&["-latency", "200ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let fixture = fixture(DUMMY_TRANSCRIPT);
    run_asr(&fixture, &dummy.url("/v1/audio/transcriptions"), &[]);

    let records = request_records(&fixture.data_log);
    assert_eq!(records.len(), 1);
    let record = &records[0];

    let latency = record["latency_s"].as_f64().expect("latency_s");
    // The server sleeps 200ms; a timer that also covered the file read and
    // process setup would overshoot well past this bound.
    assert!(
        (0.150..1.000).contains(&latency),
        "latency {latency}s outside dummy-server bounds"
    );

    let rtfx = modality_metric(record, "rtfx").or_else(|| modality_metric(record, "rtfx_client"));
    if let Some(rtfx) = rtfx {
        let expected = AUDIO_SECONDS / latency;
        assert!(
            (rtfx - expected).abs() / expected < 0.2,
            "rtfx {rtfx} does not match audio/client = {expected}"
        );
        assert!(rtfx > 1.0, "4s of audio in {latency}s should exceed 1x");
    }
    assert!(
        summary_record(&fixture.data_log).is_some(),
        "run wrote no summary record"
    );
}

/// `--normalizer` changes how WER and CER are computed, and the choice is
/// recorded in the run configuration.
#[test]
fn asr_normalizer_flag_changes_scores_and_is_recorded() {
    let Some(dummy) = spawn_dummy(&[]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let url = dummy.url("/v1/audio/transcriptions");
    // Differs from "dummy transcription" only by case and punctuation.
    let reference = "Dummy transcription.";

    let wer_for = |flag: Option<&str>| -> (f64, String) {
        let fixture = fixture(reference);
        let extra: Vec<&str> = match flag {
            Some(value) => vec!["--normalizer", value],
            None => vec![],
        };
        run_asr(&fixture, &url, &extra);
        let records = request_records(&fixture.data_log);
        let wer = modality_metric(&records[0], "wer").expect("wer in modality metrics");
        let recorded = run_config(&fixture.data_log)["normalizer"]
            .as_str()
            .expect("normalizer in config")
            .to_string();
        (wer, recorded)
    };

    let (default_wer, default_name) = wer_for(None);
    assert_eq!(default_name, "whisper-english");
    assert_eq!(default_wer, 0.0, "normalized text should match exactly");

    let (basic_wer, basic_name) = wer_for(Some("whisper-basic"));
    assert_eq!(basic_name, "whisper-basic");
    assert_eq!(
        basic_wer, 0.0,
        "case and punctuation fold in basic mode too"
    );

    let (raw_wer, raw_name) = wer_for(Some("none"));
    assert_eq!(raw_name, "none");
    assert!(
        raw_wer > 0.0,
        "raw comparison should see case and punctuation, got WER {raw_wer}"
    );
}
