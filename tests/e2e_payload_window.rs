// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! E2E (#242): the request window starts after the request body is built.
//! A large image (4096x4096 PNG) or audio file is sent to dummy-model-server
//! in strict mode, and each measured latency is compared with a reference
//! POST of the same payload to the same server, timed by this test. Client
//! side body work (base64 data URLs, JSON serialization, multipart assembly)
//! done inside the window would push the tool's latency well above the
//! reference. Skipped when `go` is missing (CI sets
//! `METRUM_BENCH_REQUIRE_DUMMY=1`). Metrum AI.
//!
//! The vlm and strategic vlm tests fail on 7b4686d (before #242): JSON
//! encoding the 11 MB image inside the window adds about 0.4 s to 0.9 s in a
//! debug build. A release build encodes it in tens of milliseconds, inside
//! the slack, so these tests catch the regression under the default debug
//! test profile only. The ASR test is a guard, not a reproduction: ASR's
//! form was already built from shared bytes with no copy, so it passes
//! before and after #242 and pins the window to a raw upload of the file.
//! The timing tests hold one lock so they never share the CPU.

mod common;

use base64::Engine;
use common::{request_records, sine_wav, skip, spawn_dummy, Dummy};
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// Dummy server latency per request.
const DUMMY_LATENCY: &str = "100ms";
/// Measured requests per tool run and reference POSTs per test.
const REQUESTS: usize = 3;
/// dummy-model-server caps request bodies at 16 MiB; stay under it.
const MAX_BODY: usize = 15 << 20;

/// One timing test at a time: tool and reference runs see the same load.
fn timing_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The large PNG, encoded once per test binary.
fn large_png() -> &'static [u8] {
    static PNG: OnceLock<Vec<u8>> = OnceLock::new();
    PNG.get_or_init(encode_large_png)
}

/// A 4096x4096 RGB PNG whose top rows are noise (incompressible) and the
/// rest flat, so it is large on the wire (about 11 MB) yet fits the dummy
/// server's body limit once base64 encoded.
fn encode_large_png() -> Vec<u8> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    use image::ImageEncoder;
    const SIDE: u32 = 4096;
    const NOISE_ROWS: u32 = 880;
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut pixels = vec![0u8; (SIDE * SIDE * 3) as usize];
    let noise = (SIDE * NOISE_ROWS * 3) as usize;
    for byte in &mut pixels[..noise] {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        *byte = state as u8;
    }
    pixels[noise..].fill(96);
    let mut png = Vec::new();
    PngEncoder::new_with_quality(&mut png, CompressionType::Default, FilterType::NoFilter)
        .write_image(&pixels, SIDE, SIDE, image::ExtendedColorType::Rgb8)
        .expect("encode png");
    png
}

/// The chat body the VLM tools send for one image (same shape and size).
fn vlm_body(png: &[u8]) -> bytes::Bytes {
    let data_url = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    );
    let body = json!({
        "model": "dummy",
        "messages": [
            {"role": "system", "content": "You are a helpful assistant capable of understanding images."},
            {"role": "user", "content": [
                {"type": "text", "text": "Describe this image"},
                {"type": "image_url", "image_url": {"url": data_url, "detail": "low"}}
            ]}
        ],
        "max_tokens": 8,
        "temperature": 0.1,
        "stream": false
    });
    let bytes = serde_json::to_vec(&body).expect("body");
    assert!(bytes.len() < MAX_BODY, "body {} bytes", bytes.len());
    bytes::Bytes::from(bytes)
}

/// Median of the reference POSTs: payload upload, server parse and media
/// validation, and the dummy latency, with no client body work timed.
fn reference_latency(mut post: impl FnMut() -> reqwest::blocking::Response) -> f64 {
    // One untimed request opens the connection, as the tools' warm runs do.
    let response = post();
    assert!(response.status().is_success(), "{}", response.status());
    let mut samples: Vec<f64> = (0..REQUESTS)
        .map(|_| {
            let start = Instant::now();
            let response = post();
            let status = response.status();
            let _ = response.bytes().expect("reference body");
            assert!(status.is_success(), "reference POST: {status}");
            start.elapsed().as_secs_f64()
        })
        .collect();
    median(&mut samples)
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

/// The tool may add connection and scheduling noise, but not the body build:
/// allow 1.25x the reference plus 100 ms. Encoding the 11 MB image inside
/// the window adds about 0.4 s (strategic) to 0.9 s (vlm) in a debug build.
fn assert_window_excludes_body(what: &str, mut latencies: Vec<f64>, reference: f64) {
    assert_eq!(latencies.len(), REQUESTS, "{what}: {latencies:?}");
    let measured = median(&mut latencies);
    let limit = reference * 1.25 + 0.1;
    eprintln!("{what}: median latency {measured:.3} s, reference {reference:.3} s");
    assert!(
        measured < limit,
        "{what}: median latency {measured:.3} s exceeds {limit:.3} s \
         (reference POST {reference:.3} s): body work is inside the request window"
    );
}

fn client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .expect("client")
}

fn ok_latencies(data_log: &Path) -> Vec<f64> {
    request_records(data_log)
        .iter()
        .map(|record| {
            assert!(record["error"].is_null(), "{record}");
            record["latency_s"].as_f64().expect("latency_s")
        })
        .collect()
}

fn run(command: &mut Command, what: &str) {
    let output = command.output().expect(what);
    assert!(
        output.status.success(),
        "{what} failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn dummy() -> Option<Dummy> {
    spawn_dummy(&["-latency", DUMMY_LATENCY])
}

fn write_vlm_prompts(dir: &Path) -> std::path::PathBuf {
    let image = dir.join("large.png");
    std::fs::write(&image, large_png()).expect("write png");
    let prompts = dir.join("prompts.jsonl");
    std::fs::write(
        &prompts,
        format!(
            "{}\n",
            json!({"prompt": "Describe this image", "image_url": image.to_str().unwrap()})
        ),
    )
    .expect("write prompts");
    prompts
}

fn vlm_reference(dummy: &Dummy) -> f64 {
    let body = vlm_body(large_png());
    let client = client();
    let url = dummy.url("/v1/chat/completions");
    reference_latency(|| {
        client
            .post(&url)
            .bearer_auth("dummy")
            .header("Content-Type", "application/json")
            .body(body.clone())
            .send()
            .expect("reference POST")
    })
}

#[test]
fn vlm_request_window_excludes_body_encoding() {
    let Some(dummy) = dummy() else {
        skip("go dummy-model-server not available");
        return;
    };
    let _timing = timing_lock();
    let dir = tempfile::tempdir().expect("tmpdir");
    let prompts = write_vlm_prompts(dir.path());
    let data_log = dir.path().join("vlm.jsonl");
    run(
        Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-vlm")).args([
            "--url",
            &dummy.url("/v1/chat/completions"),
            "--api-key",
            "dummy",
            "--scenario",
            "e2e-payload-window",
            "--num-requests",
            &(REQUESTS + 1).to_string(),
            "--warmup-requests",
            "1",
            "--concurrency",
            "1",
            "--prompts",
            prompts.to_str().unwrap(),
            "--model",
            "dummy",
            "--max-tokens",
            "8",
            "--data-log",
            data_log.to_str().unwrap(),
            "--log-level",
            "error",
        ]),
        "vlm",
    );
    let latencies: Vec<f64> = request_records(&data_log)
        .iter()
        .filter(|record| record["phase"] != "warmup")
        .map(|record| {
            assert!(record["error"].is_null(), "{record}");
            record["latency_s"].as_f64().expect("latency_s")
        })
        .collect();
    let reference = vlm_reference(&dummy);
    assert_window_excludes_body("vlm", latencies, reference);
}

#[test]
fn strategic_vlm_request_window_excludes_body_encoding() {
    let Some(dummy) = dummy() else {
        skip("go dummy-model-server not available");
        return;
    };
    let _timing = timing_lock();
    let dir = tempfile::tempdir().expect("tmpdir");
    let prompts = write_vlm_prompts(dir.path());
    let csv = dir.path().join("requests.csv");
    run(
        Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic")).args([
            "--url",
            &dummy.url("/v1/chat/completions"),
            "--model",
            "dummy",
            "--api-key",
            "dummy",
            "--kind",
            "vlm",
            "--prompts",
            prompts.to_str().unwrap(),
            "--max-tokens",
            "8",
            "--sweep",
            "1",
            "--requests-per-stage",
            &REQUESTS.to_string(),
            "--warmup-requests",
            "1",
            "--csv",
            csv.to_str().unwrap(),
            "--html",
            dir.path().join("report.html").to_str().unwrap(),
        ]),
        "strategic vlm",
    );
    let text = std::fs::read_to_string(&csv).expect("csv");
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().expect("header").split(',').collect();
    let column = |name: &str| {
        header
            .iter()
            .position(|h| *h == name)
            .unwrap_or_else(|| panic!("csv column {name}: {header:?}"))
    };
    let (latency, error, warmup) = (
        column("service_latency_s"),
        column("error"),
        column("warmup"),
    );
    let latencies: Vec<f64> = lines
        .map(|line| line.split(',').collect::<Vec<&str>>())
        .filter(|fields| fields[warmup] != "true")
        .map(|fields| {
            assert!(fields[error].is_empty(), "{fields:?}");
            fields[latency].parse().expect("service_latency_s")
        })
        .collect();
    let reference = vlm_reference(&dummy);
    assert_window_excludes_body("strategic vlm", latencies, reference);
}

/// Guard (passes before and after #242): the ASR window matches a raw
/// upload of the same 12.8 MB file.
#[test]
fn asr_request_window_excludes_form_build() {
    let Some(dummy) = dummy() else {
        skip("go dummy-model-server not available");
        return;
    };
    let _timing = timing_lock();
    let dir = tempfile::tempdir().expect("tmpdir");
    // 16 kHz mono 16-bit: 400 s is about 12.8 MB, under the body limit.
    let wav = bytes::Bytes::from(sine_wav(400.0));
    assert!(wav.len() < MAX_BODY, "wav {} bytes", wav.len());
    let audio = dir.path().join("large.wav");
    std::fs::write(&audio, &wav).expect("write wav");
    let input = dir.path().join("input.jsonl");
    std::fs::write(
        &input,
        format!(
            "{}\n",
            json!({"id": "large", "path": audio.to_str().unwrap(), "format": "wav", "duration": 400.0})
        ),
    )
    .expect("write input");
    let data_log = dir.path().join("asr.jsonl");
    run(
        Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-asr")).args([
            "--url",
            &dummy.url("/v1/audio/transcriptions"),
            "--api-key",
            "dummy",
            "--scenario",
            "e2e-payload-window",
            "--num-requests",
            &REQUESTS.to_string(),
            "--concurrency",
            "1",
            "--input",
            input.to_str().unwrap(),
            "--model",
            "dummy",
            "--data-log",
            data_log.to_str().unwrap(),
            "--log-level",
            "error",
        ]),
        "asr",
    );
    let latencies = ok_latencies(&data_log);
    let client = client();
    let url = dummy.url("/v1/audio/transcriptions");
    let reference = reference_latency(|| {
        let part = reqwest::blocking::multipart::Part::reader_with_length(
            std::io::Cursor::new(wav.clone()),
            wav.len() as u64,
        )
        .file_name("large.wav")
        .mime_str("audio/wav")
        .expect("mime");
        let form = reqwest::blocking::multipart::Form::new()
            .part("file", part)
            .text("model", "dummy")
            .text("response_format", "json")
            .text("timestamp_granularities[]", "word");
        client
            .post(&url)
            .bearer_auth("dummy")
            .multipart(form)
            .send()
            .expect("reference POST")
    });
    assert_window_excludes_body("asr", latencies, reference);
}

/// Guard: the dummy rejects bodies over its limit, so the fixtures above
/// must stay under it or every request would fail before the comparison.
#[test]
fn fixtures_fit_the_dummy_body_limit() {
    let body = vlm_body(large_png());
    let parsed: Value = serde_json::from_slice(&body).expect("json");
    assert!(parsed["messages"][1]["content"][1]["image_url"]["url"]
        .as_str()
        .is_some_and(|url| url.starts_with("data:image/png;base64,")));
    assert!(sine_wav(400.0).len() < MAX_BODY);
}
