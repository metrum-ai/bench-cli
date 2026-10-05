// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! E2E (#197): strategic `--kind vlm`, `--kind asr` and `--kind imagegen`
//! sweeps against dummy-model-server in strict mode, with `--telemetry`
//! scraping the mock `--telemetry-fixture` page and `--ndjson`. Each sweep
//! must carry stage summaries with the modality metrics and
//! `knee_detection`. Skipped when `go` is missing (CI sets
//! `METRUM_BENCH_REQUIRE_DUMMY=1`). Metrum AI.

mod common;

use common::{sine_wav, skip, spawn_dummy, spawn_mock, tiny_png};
use serde_json::Value;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Command;

const STAGES: &[&str] = &["1", "2", "3", "4", "5"];
const REQUESTS: u64 = 4;
const WARMUP: u64 = 1;

struct Sweep {
    dir: tempfile::TempDir,
    ndjson: PathBuf,
}

impl Sweep {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tmpdir");
        Self {
            ndjson: dir.path().join("run.ndjson"),
            dir,
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.path(name);
        std::fs::write(&path, bytes).expect("write fixture");
        path
    }
}

fn telemetry_yaml(sweep: &Sweep, metrics: SocketAddr) -> PathBuf {
    sweep.write(
        "telemetry.yaml",
        format!(
            "timeout_ms: 500\nsources:\n  - name: all-smi\n    url: http://{metrics}/metrics\n    interval_ms: 100\n    include:\n      - \"^all_smi_(gpu|cpu|memory)_\"\n"
        )
        .as_bytes(),
    )
}

/// Run one sweep and return the stdout summary.
fn run_sweep(sweep: &Sweep, url: &str, kind: &str, metrics: SocketAddr, extra: &[&str]) -> Value {
    let yaml = telemetry_yaml(sweep, metrics);
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"))
        .args([
            "--url",
            url,
            "--model",
            "dummy",
            "--api-key",
            "dummy",
            "--kind",
            kind,
            "--sweep",
            &STAGES.join(","),
            "--requests-per-stage",
            &REQUESTS.to_string(),
            "--warmup-requests",
            &WARMUP.to_string(),
            "--ndjson",
            sweep.ndjson.to_str().unwrap(),
            "--telemetry",
            yaml.to_str().unwrap(),
            "--require-telemetry",
            "--html",
            sweep.path("report.html").to_str().unwrap(),
            "--csv",
            sweep.path("requests.csv").to_str().unwrap(),
        ])
        .args(extra)
        .output()
        .expect("run strategic");
    assert!(
        output.status.success(),
        "{kind} sweep failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("summary JSON")
}

/// Shared acceptance: every stage measured with no errors, modality keys
/// present, knee detection present, and a complete telemetry NDJSON.
fn assert_sweep(sweep: &Sweep, summary: &Value, kind: &str, keys: &[&str]) {
    assert_eq!(summary["config"]["kind"], kind);
    assert!(summary["config"]["modality"].is_object());
    let points = summary["points"].as_array().expect("points");
    assert_eq!(points.len(), STAGES.len());
    for point in points {
        assert_eq!(point["n"], REQUESTS, "{kind} point {point}");
        assert_eq!(point["errors"], 0, "{kind} point {point}");
        assert!(point["throughput"].as_f64().unwrap() > 0.0);
        assert!(point["latency_s"]["n"].as_u64().unwrap() > 0);
        let metrics = point["modality_metrics"]
            .as_object()
            .expect("modality_metrics");
        let got: Vec<&str> = metrics.keys().map(String::as_str).collect();
        assert_eq!(got, keys, "{kind} modality keys");
    }
    let knee = &summary["knee_detection"];
    assert!(knee.is_object(), "knee_detection: {summary}");
    assert!(knee.get("index").is_some());
    assert!(
        knee["reason"].is_string() || knee["index"].is_u64(),
        "{knee}"
    );

    let rows: Vec<Value> = std::fs::read_to_string(&sweep.ndjson)
        .expect("ndjson")
        .lines()
        .map(|line| serde_json::from_str(line).expect("ndjson row"))
        .collect();
    let mut kinds = BTreeMap::<String, u64>::new();
    for row in &rows {
        *kinds
            .entry(row["kind"].as_str().unwrap().to_string())
            .or_default() += 1;
    }
    assert_eq!(rows[0]["kind"], "run");
    assert_eq!(rows[0]["config"]["kind"], kind);
    // NDJSON-only analysis sees the same kind settings as stdout.
    assert_eq!(rows[0]["config"]["modality"], summary["config"]["modality"]);
    assert_eq!(
        rows[0]["config"]["temperature"],
        summary["config"]["temperature"]
    );
    let per_stage = REQUESTS + WARMUP;
    assert_eq!(
        kinds["request"],
        per_stage * STAGES.len() as u64,
        "{kinds:?}"
    );
    assert_eq!(kinds["stage"], 2 * STAGES.len() as u64, "{kinds:?}");
    assert!(
        kinds.get("telemetry").copied().unwrap_or(0) > 0,
        "{kinds:?}"
    );
    assert_eq!(kinds.get("scrape_error"), None, "{kinds:?}");
    let last = rows.last().unwrap();
    assert_eq!(last["kind"], "summary");
    assert_eq!(last["partial"], false);
    assert!(rows
        .iter()
        .filter(|row| row["kind"] == "request")
        .all(|row| row["success"] == true));
}

fn start() -> Option<(common::Mock, common::Dummy)> {
    let mock = spawn_mock(&["--telemetry-fixture"]);
    let Some(dummy) = spawn_dummy(&["-latency", "20ms"]) else {
        skip("go dummy-model-server not available");
        return None;
    };
    Some((mock, dummy))
}

#[test]
fn strategic_vlm_sweep_with_telemetry() {
    let Some((mock, dummy)) = start() else { return };
    let sweep = Sweep::new();
    let png = tiny_png();
    let image = sweep.write("tiny.png", &png);
    let summary = run_sweep(
        &sweep,
        &dummy.url("/v1/chat/completions"),
        "vlm",
        mock.address,
        &[
            "--image",
            image.to_str().unwrap(),
            "--prompt",
            "Describe the image.",
            "--max-tokens",
            "8",
            "--streaming",
        ],
    );
    assert_sweep(&sweep, &summary, "vlm", &["image_bytes", "image_count"]);
    for point in summary["points"].as_array().unwrap() {
        let metrics = &point["modality_metrics"];
        assert_eq!(metrics["image_count"]["n"], REQUESTS);
        assert_eq!(metrics["image_count"]["max"], 1.0);
        assert_eq!(metrics["image_bytes"]["max"], png.len() as f64);
        // VLM keeps the chat timing: streamed TTFT and server token usage.
        assert_eq!(point["ttft_s"]["n"], REQUESTS);
        assert!(point["isl_tokens"]["n"].as_u64().unwrap() > 0);
        assert!(point.get("image_digests").is_none());
    }
}

#[test]
fn strategic_asr_sweep_with_telemetry() {
    let Some((mock, dummy)) = start() else { return };
    let sweep = Sweep::new();
    let wav = sweep.write("tone.wav", &sine_wav(0.5));
    let samples = sweep.write(
        "audio.jsonl",
        format!(
            "{{\"id\":\"tone\",\"path\":\"{}\",\"format\":\"wav\",\"duration\":0.5}}\n",
            wav.display()
        )
        .as_bytes(),
    );
    // dummy-model-server always transcribes "dummy transcription".
    let references = sweep.write(
        "refs.jsonl",
        b"{\"id\":\"tone\",\"transcript\":\"Dummy transcription.\"}\n",
    );
    let summary = run_sweep(
        &sweep,
        &dummy.url("/v1/audio/transcriptions"),
        "asr",
        mock.address,
        &[
            "--audio-samples",
            samples.to_str().unwrap(),
            "--ground-truth",
            references.to_str().unwrap(),
        ],
    );
    assert_sweep(
        &sweep,
        &summary,
        "asr",
        &["audio_duration_s", "cer", "rtfx_client", "wer"],
    );
    assert_eq!(summary["config"]["modality"]["references_matched"], 1);
    for point in summary["points"].as_array().unwrap() {
        let metrics = &point["modality_metrics"];
        assert_eq!(metrics["wer"]["n"], REQUESTS);
        assert_eq!(metrics["wer"]["max"], 0.0);
        assert_eq!(metrics["cer"]["max"], 0.0);
        assert_eq!(metrics["audio_duration_s"]["min"], 0.5);
        assert_eq!(metrics["rtfx_client"]["n"], REQUESTS);
        assert!(metrics["rtfx_client"]["min"].as_f64().unwrap() > 0.0);
        // Unary uploads: no TTFT, never fabricated.
        assert_eq!(point["ttft_s"]["n"], 0);
    }
}

#[test]
fn strategic_asr_without_references_reports_n_zero() {
    let Some((mock, dummy)) = start() else { return };
    let sweep = Sweep::new();
    let wav = sweep.write("tone.wav", &sine_wav(0.25));
    let samples = sweep.write(
        "audio.jsonl",
        format!(
            "{{\"id\":\"tone\",\"path\":\"{}\",\"format\":\"wav\"}}\n",
            wav.display()
        )
        .as_bytes(),
    );
    let summary = run_sweep(
        &sweep,
        &dummy.url("/v1/audio/transcriptions"),
        "asr",
        mock.address,
        &[
            "--audio-samples",
            samples.to_str().unwrap(),
            "--asr-response-format",
            "text",
        ],
    );
    for point in summary["points"].as_array().unwrap() {
        let metrics = &point["modality_metrics"];
        for key in ["wer", "cer", "rtfx_client", "audio_duration_s"] {
            assert_eq!(metrics[key]["n"], 0, "{key} must be n = 0: {metrics}");
            assert!(metrics[key]["p50"].is_null());
        }
    }
}

#[test]
fn strategic_imagegen_sweep_with_telemetry() {
    let Some((mock, dummy)) = start() else { return };
    let sweep = Sweep::new();
    let prompts = sweep.write(
        "prompts.jsonl",
        b"{\"prompt\":\"a red square\"}\n{\"prompt\":\"a blue circle\"}\n",
    );
    let summary = run_sweep(
        &sweep,
        &dummy.url("/v1/images/generations"),
        "imagegen",
        mock.address,
        &[
            "--prompts",
            prompts.to_str().unwrap(),
            "--image-size",
            "64x64",
            "--images-per-request",
            "2",
        ],
    );
    assert_sweep(
        &sweep,
        &summary,
        "imagegen",
        &["images_requested", "images_returned"],
    );
    for point in summary["points"].as_array().unwrap() {
        let metrics = &point["modality_metrics"];
        assert_eq!(metrics["images_requested"]["max"], 2.0);
        assert_eq!(metrics["images_returned"]["min"], 2.0);
        // Two prompts x two images, deterministic per prompt and index.
        assert_eq!(
            point["image_digests"],
            serde_json::json!({"images": REQUESTS * 2, "distinct": 4})
        );
        assert_eq!(point["osl_tokens"]["n"], 0);
    }
}

#[test]
fn strategic_imagegen_url_responses_have_no_digests() {
    let Some((mock, dummy)) = start() else { return };
    let sweep = Sweep::new();
    let summary = run_sweep(
        &sweep,
        &dummy.url("/v1/images/generations"),
        "imagegen",
        mock.address,
        &[
            "--prompt",
            "a green square",
            "--image-size",
            "64x64",
            "--image-response-format",
            "url",
        ],
    );
    for point in summary["points"].as_array().unwrap() {
        assert_eq!(point["modality_metrics"]["images_returned"]["min"], 1.0);
        assert_eq!(
            point["image_digests"],
            serde_json::json!({"images": 0, "distinct": 0})
        );
    }
}

#[test]
fn chat_sweep_output_has_no_modality_fields() {
    let mock = spawn_mock(&["--latency-ms", "5"]);
    let sweep = Sweep::new();
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"))
        .args([
            "--url",
            &format!("http://{}/v1/chat/completions", mock.address),
            "--model",
            "mock",
            "--api-key",
            "dummy",
            "--max-tokens",
            "8",
            "--sweep",
            "1,2",
            "--requests-per-stage",
            "2",
            "--html",
            sweep.path("report.html").to_str().unwrap(),
            "--csv",
            sweep.path("requests.csv").to_str().unwrap(),
        ])
        .output()
        .expect("run strategic");
    assert!(output.status.success());
    let summary: Value = serde_json::from_slice(&output.stdout).expect("summary");
    assert!(summary["config"].get("modality").is_none());
    assert!(summary["config"].get("temperature").is_none());
    for point in summary["points"].as_array().unwrap() {
        assert!(point.get("modality_metrics").is_none());
        assert!(point.get("image_digests").is_none());
    }
}

#[test]
fn modality_flags_rejected_for_other_kinds() {
    let cases: &[(&[&str], &str)] = &[
        (&["--kind", "chat", "--image", "x.png"], "--image"),
        (
            &["--kind", "embeddings", "--audio-samples", "a.jsonl"],
            "--audio-samples",
        ),
        (
            &["--kind", "asr", "--prompts", "p.jsonl"],
            "--audio-samples",
        ),
        (&["--kind", "asr"], "--audio-samples"),
        (&["--kind", "vlm", "--image", "x.png"], "--max-tokens"),
        (&["--kind", "imagegen", "--ignore-eos"], "--ignore-eos"),
        (
            &["--kind", "rerank", "--temperature", "0.5"],
            "--temperature",
        ),
        (
            &["--kind", "vlm", "--json-schema", "s.json"],
            "--json-schema",
        ),
        (
            &["--kind", "imagegen", "--sessions", "s.jsonl"],
            "--sessions",
        ),
        (
            &["--kind", "asr", "--streaming", "--audio-samples", "a.jsonl"],
            "--streaming",
        ),
        (&["--kind", "imagegen", "--max-tokens", "8"], "--max-tokens"),
        (
            &["--kind", "chat", "--max-image-dimension", "64"],
            "--max-image-dimension",
        ),
        (
            &[
                "--kind",
                "vlm",
                "--max-tokens",
                "8",
                "--prompts",
                "p.jsonl",
                "--image",
                "x.png",
            ],
            "not with --prompts",
        ),
        (
            &[
                "--kind",
                "vlm",
                "--max-tokens",
                "8",
                "--image",
                "x.png",
                "--shared-prefix",
                "ctx",
            ],
            "--shared-prefix",
        ),
    ];
    for (flags, needle) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"))
            .args([
                "--url",
                "http://127.0.0.1:9/v1",
                "--model",
                "m",
                "--api-key",
                "dummy",
            ])
            .args(*flags)
            .output()
            .expect("run strategic");
        assert!(!output.status.success(), "{flags:?} must fail");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(needle), "{flags:?}: {stderr}");
    }
}
