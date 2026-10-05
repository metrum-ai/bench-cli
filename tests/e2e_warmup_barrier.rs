// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! E2E (#226): warmup is a barrier in every Metrum AI Bench load binary.
//! Measured requests start only after every warmup request has completed,
//! so each measured `t_sent_ns` is at or after the last warmup `t_done_ns`
//! and the `warmup` and `measure` stage windows do not overlap. Modality
//! runs use dummy-model-server in strict mode and are skipped when `go` is
//! missing (CI sets `METRUM_BENCH_REQUIRE_DUMMY=1`); strategic uses the Rust
//! mock.

mod common;

use common::{request_records, sine_wav, skip, spawn_dummy, spawn_mock, summary_record, tiny_png};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Concurrency above the warmup count, so without a barrier measured
/// requests would be sent while warmup is still in flight.
const CONCURRENCY: &str = "4";
const WARMUP: u64 = 2;
const REQUESTS: u64 = 8;

struct Run {
    dir: tempfile::TempDir,
    data_log: PathBuf,
    ndjson: PathBuf,
}

fn new_run() -> Run {
    let dir = tempfile::tempdir().expect("tmpdir");
    Run {
        data_log: dir.path().join("out.jsonl"),
        ndjson: dir.path().join("run.ndjson"),
        dir,
    }
}

/// Flags every modality run shares: load shape, outputs, quiet logs.
fn common_flags(run: &Run) -> Vec<String> {
    let dir = run.dir.path();
    [
        "--api-key",
        "dummy",
        "--scenario",
        "e2e-warmup-barrier",
        "--num-requests",
        &REQUESTS.to_string(),
        "--warmup-requests",
        &WARMUP.to_string(),
        "--concurrency",
        CONCURRENCY,
        "--data-log",
        run.data_log.to_str().unwrap(),
        "--ndjson",
        run.ndjson.to_str().unwrap(),
        "--debug-log",
        dir.join("debug.log").to_str().unwrap(),
        "--error-log",
        dir.join("error.log").to_str().unwrap(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn assert_ok(output: &Output, what: &str) {
    assert!(
        output.status.success(),
        "{what} failed: stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
}

fn ndjson_rows(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .expect("read ndjson")
        .lines()
        .map(|line| serde_json::from_str(line).expect("ndjson row"))
        .collect()
}

fn ns(row: &Value, key: &str) -> u64 {
    row[key]
        .as_u64()
        .unwrap_or_else(|| panic!("{key} missing: {row}"))
}

/// Barrier acceptance for one stage: every measured send is at or after the
/// last warmup completion, and the stage windows do not overlap.
fn assert_barrier(requests: &[&Value], stages: &[&Value], warmup: u64, measured: u64) {
    let warm: Vec<&&Value> = requests.iter().filter(|r| r["warmup"] == true).collect();
    let meas: Vec<&&Value> = requests.iter().filter(|r| r["warmup"] == false).collect();
    assert_eq!(warm.len() as u64, warmup, "warmup rows");
    assert_eq!(meas.len() as u64, measured, "measured rows");
    let last_warmup_done = warm
        .iter()
        .map(|r| ns(r, "t_done_ns"))
        .max()
        .expect("warmup");
    for row in &meas {
        assert!(
            ns(row, "t_sent_ns") >= last_warmup_done,
            "measured request sent before warmup drained: {row} (last warmup done {last_warmup_done})"
        );
    }
    let stage = |phase: &str| {
        *stages
            .iter()
            .find(|s| s["phase"] == phase)
            .unwrap_or_else(|| panic!("{phase} stage row"))
    };
    let (warm_stage, meas_stage) = (stage("warmup"), stage("measure"));
    assert!(
        ns(warm_stage, "t_end_ns") <= ns(meas_stage, "t_start_ns"),
        "stage windows overlap: {warm_stage} vs {meas_stage}"
    );
}

/// Modality runs: one stage, so every request and stage row is checked
/// together. The data log agrees: each measured `send_offset_s` is at or
/// after every warmup `send_offset_s + latency_s`.
fn assert_modality_barrier(run: &Run) {
    let rows = ndjson_rows(&run.ndjson);
    let requests: Vec<&Value> = rows.iter().filter(|r| r["kind"] == "request").collect();
    let stages: Vec<&Value> = rows.iter().filter(|r| r["kind"] == "stage").collect();
    assert_eq!(requests.len() as u64, REQUESTS);
    assert!(
        requests.iter().all(|r| r["success"] == true),
        "{requests:?}"
    );
    assert_barrier(&requests, &stages, WARMUP, REQUESTS - WARMUP);

    let records = request_records(&run.data_log);
    let offset = |r: &Value| r["send_offset_s"].as_f64().expect("send_offset_s");
    let warm_done = records
        .iter()
        .filter(|r| r["phase"] == "warmup")
        .map(|r| offset(r) + r["latency_s"].as_f64().expect("latency_s"))
        .fold(f64::NEG_INFINITY, f64::max);
    // 1 us tolerance for the f64 seconds round trip.
    for r in records.iter().filter(|r| r["phase"] == "measure") {
        assert!(offset(r) + 1e-6 >= warm_done, "data log: {r}");
    }
    let summary = summary_record(&run.data_log).expect("summary");
    assert_measured_only(&summary["observed_concurrency"], REQUESTS - WARMUP);
}

/// The tracker resets at the barrier, so `observed_concurrency` counts the
/// measured slots only: one acquire per measured request, none for warmup.
fn assert_measured_only(observed: &Value, measured: u64) {
    assert_eq!(
        observed["acquire_count"].as_u64(),
        Some(measured),
        "observed_concurrency counts warmup: {observed}"
    );
    let waits = observed["wait_count"].as_u64().expect("wait_count");
    assert!(waits <= measured, "{observed}");
}

#[test]
fn llm_measured_requests_wait_for_warmup() {
    let Some(dummy) = spawn_dummy(&["-latency", "100ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let run = new_run();
    let prompts = run.dir.path().join("prompts.jsonl");
    std::fs::write(&prompts, "{\"prompt\":\"hello barrier\"}\n").expect("write prompts");
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-llm"))
        .args([
            "--url",
            &dummy.url("/v1/chat/completions"),
            "--prompts",
            prompts.to_str().unwrap(),
            "--mode",
            "chat",
            "--streaming",
            "--model",
            "dummy",
            "--max-tokens",
            "8",
            "--log-level",
            "error",
        ])
        .args(common_flags(&run))
        .output()
        .expect("run llm");
    assert_ok(&output, "llm");
    assert_modality_barrier(&run);
}

/// Open loop: the barrier shifts the measured schedule instead of letting
/// measured requests burst to catch up on time spent in warmup.
#[test]
fn llm_open_loop_measured_requests_wait_for_warmup() {
    let Some(dummy) = spawn_dummy(&["-latency", "200ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let run = new_run();
    let prompts = run.dir.path().join("prompts.jsonl");
    std::fs::write(&prompts, "{\"prompt\":\"hello barrier\"}\n").expect("write prompts");
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-llm"))
        .args([
            "--url",
            &dummy.url("/v1/chat/completions"),
            "--prompts",
            prompts.to_str().unwrap(),
            "--mode",
            "chat",
            "--model",
            "dummy",
            "--max-tokens",
            "8",
            "--request-rate",
            "20",
            "--arrival",
            "constant",
            "--log-level",
            "error",
        ])
        .args(common_flags(&run))
        .output()
        .expect("run llm");
    assert_ok(&output, "llm open loop");
    assert_modality_barrier(&run);

    // Constant 20 req/s: the seeded 50 ms gaps survive the shift, the first
    // measured slot is due no earlier than warmup drained, and no measured
    // request inherits warmup time as queue delay. Without the shift every
    // measured slot would be overdue at the barrier.
    let records = request_records(&run.data_log);
    let warm_done = records
        .iter()
        .filter(|r| r["phase"] == "warmup")
        .map(|r| {
            r["send_offset_s"].as_f64().expect("send_offset_s")
                + r["latency_s"].as_f64().expect("latency_s")
        })
        .fold(f64::NEG_INFINITY, f64::max);
    let mut measured: Vec<&Value> = records.iter().filter(|r| r["phase"] == "measure").collect();
    measured.sort_by_key(|r| r["seq"].as_u64().expect("seq"));
    let sched: Vec<f64> = measured
        .iter()
        .map(|r| {
            r["scheduled_offset_s"]
                .as_f64()
                .expect("scheduled_offset_s")
        })
        .collect();
    for pair in sched.windows(2) {
        assert!(
            ((pair[1] - pair[0]) - 0.05).abs() < 1e-6,
            "measured schedule gap must stay 50 ms: {sched:?}"
        );
    }
    assert!(
        sched[0] + 1e-6 >= warm_done,
        "first measured slot due before warmup drained: {} < {warm_done}",
        sched[0]
    );
    for r in &measured {
        let queue = r["queue_delay_s"].as_f64().unwrap_or(0.0);
        assert!(queue < 0.05, "measured queue delay counts warmup: {r}");
    }
}

#[test]
fn vlm_measured_requests_wait_for_warmup() {
    let Some(dummy) = spawn_dummy(&["-latency", "100ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let run = new_run();
    let image = run.dir.path().join("pixel.png");
    std::fs::write(&image, tiny_png()).expect("write png");
    let prompts = run.dir.path().join("prompts.jsonl");
    std::fs::write(
        &prompts,
        format!(
            "{{\"prompt\":\"Describe this image\",\"image_url\":\"{}\"}}\n",
            image.to_str().unwrap()
        ),
    )
    .expect("write prompts");
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-vlm"))
        .args([
            "--url",
            &dummy.url("/v1/chat/completions"),
            "--prompts",
            prompts.to_str().unwrap(),
            "--model",
            "dummy",
            "--max-tokens",
            "8",
            "--log-level",
            "error",
        ])
        .args(common_flags(&run))
        .output()
        .expect("run vlm");
    assert_ok(&output, "vlm");
    assert_modality_barrier(&run);
}

#[test]
fn asr_measured_requests_wait_for_warmup() {
    let Some(dummy) = spawn_dummy(&["-latency", "100ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let run = new_run();
    let audio = run.dir.path().join("sample.wav");
    std::fs::write(&audio, sine_wav(1.0)).expect("write wav");
    let input = run.dir.path().join("input.jsonl");
    std::fs::write(
        &input,
        format!(
            "{{\"id\":\"sample-1\",\"path\":\"{}\",\"format\":\"wav\",\"duration\":1.0}}\n",
            audio.to_str().unwrap()
        ),
    )
    .expect("write input");
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-asr"))
        .args([
            "--url",
            &dummy.url("/v1/audio/transcriptions"),
            "--input",
            input.to_str().unwrap(),
            "--model",
            "dummy",
            "--log-level",
            "error",
        ])
        .args(common_flags(&run))
        .output()
        .expect("run asr");
    assert_ok(&output, "asr");
    assert_modality_barrier(&run);
}

#[test]
fn imagegen_measured_requests_wait_for_warmup() {
    let Some(dummy) = spawn_dummy(&["-latency", "100ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let run = new_run();
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-imagegen"))
        .args([
            "--url",
            &dummy.url("/v1"),
            "--model",
            "dummy",
            "--prompt",
            "a small test image",
            "--size",
            "64x64",
            "--artifact-dir",
            run.dir.path().join("artifacts").to_str().unwrap(),
            "--no-save-images",
        ])
        .args(common_flags(&run))
        .output()
        .expect("run imagegen");
    assert_ok(&output, "imagegen");
    assert_modality_barrier(&run);
}

/// Strategic already drains per-stage warmup before measuring; pin it with
/// the same NDJSON check at concurrency 4 across two stages.
#[test]
fn strategic_per_stage_warmup_is_a_barrier() {
    let server = spawn_mock(&["--latency-ms", "50"]);
    let run = new_run();
    let prompts = run.dir.path().join("prompts.jsonl");
    std::fs::write(&prompts, "{\"prompt\":\"alpha\"}\n{\"prompt\":\"beta\"}\n")
        .expect("write prompts");
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"))
        .args([
            "--url",
            &format!("http://{}/v1/chat/completions", server.address),
            "--model",
            "mock",
            "--prompts",
            prompts.to_str().unwrap(),
            "--max-tokens",
            "8",
            "--warmup-requests",
            &WARMUP.to_string(),
            "--requests-per-stage",
            "6",
            "--sweep",
            "2,4",
            "--ndjson",
            run.ndjson.to_str().unwrap(),
            "--html",
            run.dir.path().join("report.html").to_str().unwrap(),
        ])
        .output()
        .expect("run strategic");
    assert_ok(&output, "strategic");
    let summary: Value = serde_json::from_slice(&output.stdout).expect("summary JSON");
    let points = summary["points"].as_array().expect("points");
    assert_eq!(points.len(), 2);
    for point in points {
        assert_measured_only(&point["observed_concurrency"], 6);
    }
    let rows = ndjson_rows(&run.ndjson);
    let mut stage_ids: Vec<String> = rows
        .iter()
        .filter(|r| r["kind"] == "stage")
        .map(|r| r["stage"].to_string())
        .collect();
    stage_ids.dedup();
    assert_eq!(stage_ids.len(), 2, "two sweep stages: {stage_ids:?}");
    for id in &stage_ids {
        let requests: Vec<&Value> = rows
            .iter()
            .filter(|r| r["kind"] == "request" && &r["stage"].to_string() == id)
            .collect();
        let stages: Vec<&Value> = rows
            .iter()
            .filter(|r| r["kind"] == "stage" && &r["stage"].to_string() == id)
            .collect();
        assert_barrier(&requests, &stages, WARMUP, 6);
    }
}
