// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! VLM end-to-end checks against dummy-model-server. Skipped if `go` is missing.

mod common;

use common::{request_records, run_config, skip, spawn_dummy, summary_record, tiny_png};
use std::io::Write;
use std::process::Command;

fn vlm_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_metrum-ai-bench-vlm"))
}

struct Fixture {
    _dir: tempfile::TempDir,
    prompts: std::path::PathBuf,
    image: std::path::PathBuf,
    data_log: std::path::PathBuf,
    debug_log: std::path::PathBuf,
    error_log: std::path::PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("tmpdir");
    let image = dir.path().join("pixel.png");
    std::fs::write(&image, tiny_png()).expect("write png");

    let prompts = dir.path().join("prompts.jsonl");
    let mut file = std::fs::File::create(&prompts).expect("create prompts");
    writeln!(
        file,
        r#"{{"prompt":"Describe this image","image_url":"{}"}}"#,
        image.to_str().unwrap()
    )
    .expect("write prompts");

    Fixture {
        prompts,
        image,
        data_log: dir.path().join("out.jsonl"),
        debug_log: dir.path().join("debug.log"),
        error_log: dir.path().join("error.log"),
        _dir: dir,
    }
}

fn run_vlm(fixture: &Fixture, url: &str, requests: u32, extra: &[&str]) {
    let requests = requests.to_string();
    let mut args: Vec<String> = [
        "--url",
        url,
        "--api-key",
        "dummy",
        "--scenario",
        "e2e-vlm",
        "--num-requests",
        &requests,
        "--concurrency",
        "1",
        "--prompts",
        fixture.prompts.to_str().unwrap(),
        "--model",
        "dummy",
        "--max-tokens",
        "20",
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

    let status = Command::new(vlm_bin())
        .args(&args)
        .status()
        .expect("run vlm");
    assert!(status.success(), "vlm bench failed");
}

/// Streaming TTFT must come from the first content token, not be fabricated,
/// and image preprocessing must sit outside the measured window.
#[test]
fn vlm_streaming_measures_real_ttft_and_itl() {
    let Some(dummy) = spawn_dummy(&["-latency", "120ms", "-chunk-interval", "20ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let fixture = fixture();
    run_vlm(
        &fixture,
        &dummy.url("/v1/chat/completions"),
        2,
        &["--streaming"],
    );

    let records = request_records(&fixture.data_log);
    assert_eq!(records.len(), 2, "expected one record per request");
    for record in &records {
        let ttft = record["ttft_s"].as_f64().expect("ttft_s");
        let latency = record["latency_s"].as_f64().expect("latency_s");
        // Server holds 120ms, then streams a chunk every 20ms.
        assert!(
            (0.100..0.400).contains(&ttft),
            "ttft {ttft}s outside dummy-server bounds"
        );
        assert!(
            latency > ttft,
            "latency {latency}s must exceed ttft {ttft}s"
        );
        // Image decode and base64 happen during preload, so a 2x2 PNG cannot
        // add measurable time; a fabricated TTFT would equal latency exactly.
        assert!(
            (latency - ttft) > 0.010,
            "ttft {ttft}s looks fabricated from latency {latency}s"
        );

        let itl: Vec<f64> = record["itl_s"]
            .as_array()
            .expect("itl_s array")
            .iter()
            .filter_map(serde_json::Value::as_f64)
            .collect();
        assert!(!itl.is_empty(), "streaming run recorded no ITL samples");
        let mean = itl.iter().sum::<f64>() / itl.len() as f64;
        assert!(
            (0.005..0.060).contains(&mean),
            "mean ITL {mean}s (dummy streams every 20ms)"
        );
    }
    assert!(
        summary_record(&fixture.data_log).is_some(),
        "run wrote no summary record"
    );
}

/// Non-streaming runs report latency without inventing a TTFT.
#[test]
fn vlm_non_streaming_reports_no_ttft() {
    let Some(dummy) = spawn_dummy(&["-latency", "50ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let fixture = fixture();
    run_vlm(&fixture, &dummy.url("/v1/chat/completions"), 2, &[]);

    for record in request_records(&fixture.data_log) {
        assert!(
            record["ttft_s"].is_null(),
            "non-streaming record reported ttft {:?}",
            record["ttft_s"]
        );
        assert!(record["latency_s"].as_f64().expect("latency_s") >= 0.050);
    }
}

/// By default the original image bytes are sent; `--reencode-jpeg` opts in to
/// re-encoding. The payload size difference makes the choice observable.
#[test]
fn vlm_sends_original_bytes_unless_reencode_requested() {
    let Some(dummy) = spawn_dummy(&[]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let url = dummy.url("/v1/chat/completions");

    let original = fixture();
    let source_bytes = std::fs::metadata(&original.image).expect("stat png").len() as f64;
    run_vlm(&original, &url, 2, &[]);

    let reencoded = fixture();
    run_vlm(&reencoded, &url, 2, &["--reencode-jpeg"]);

    let image_bytes = |data_log: &std::path::Path| -> f64 {
        let records = request_records(data_log);
        assert!(!records.is_empty(), "no request records");
        let bytes = records[0]["modality_metrics"]["image_bytes"]
            .as_f64()
            .expect("image_bytes");
        assert_eq!(
            records[0]["modality_metrics"]["image_count"].as_f64(),
            Some(1.0)
        );
        bytes
    };

    assert_eq!(
        image_bytes(&original.data_log),
        source_bytes,
        "default run must send the source bytes unchanged"
    );
    assert_ne!(
        image_bytes(&reencoded.data_log),
        source_bytes,
        "--reencode-jpeg must replace the source bytes"
    );

    assert_eq!(
        run_config(&original.data_log)["reencode_jpeg"],
        serde_json::json!(false)
    );
    assert_eq!(
        run_config(&reencoded.data_log)["reencode_jpeg"],
        serde_json::json!(true)
    );
}

/// Warmup requests must still be written to the data log (tagged `warmup`),
/// not dropped by a completion-order skip. Shared summary excludes them.
#[test]
fn vlm_warmup_requests_are_logged_not_dropped() {
    let Some(dummy) = spawn_dummy(&[]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let fixture = fixture();
    run_vlm(
        &fixture,
        &dummy.url("/v1/chat/completions"),
        3,
        &["--warmup-requests", "1"],
    );

    let records = request_records(&fixture.data_log);
    assert_eq!(
        records.len(),
        3,
        "warmup requests must still be logged as request records"
    );
    let warmup = records.iter().filter(|r| r["phase"] == "warmup").count();
    assert_eq!(warmup, 1, "expected one warmup-phase record");
    let measure = records.iter().filter(|r| r["phase"] == "measure").count();
    assert_eq!(measure, 2, "expected two measure-phase records");

    let summary = summary_record(&fixture.data_log).expect("shared summary");
    assert_eq!(
        summary["latency_s"]["n"], 2,
        "warmup request must not be measured"
    );
}

/// F-10: VLM honors --system-prompt (empty disables), --min-tokens, and stamps
/// effective_system_prompt like the LLM binary.
#[test]
fn vlm_honors_system_prompt_and_min_tokens() {
    let Some(dummy) = spawn_dummy(&[]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let fixture = fixture();
    run_vlm(
        &fixture,
        &dummy.url("/v1/chat/completions"),
        1,
        &["--system-prompt", "", "--min-tokens", "3", "--ignore-eos"],
    );

    let summary = summary_record(&fixture.data_log).expect("shared summary");
    let config = summary.get("config").expect("config");
    assert!(
        config["effective_system_prompt"].is_null(),
        "empty --system-prompt must disable the system message"
    );
    assert_eq!(config["common"]["min_tokens"], 3);
    assert_eq!(config["common"]["ignore_eos"], true);
    let body = &config["body_template"];
    let body_str = body.to_string();
    assert!(
        !body_str.contains("\"role\":\"system\"") && !body_str.contains("\"role\": \"system\""),
        "body_template must omit system message when disabled: {body_str}"
    );
    assert_eq!(body["min_tokens"], 3);
    assert_eq!(body["ignore_eos"], true);
}
