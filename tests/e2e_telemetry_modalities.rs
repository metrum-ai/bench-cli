// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! E2E (#196): llm, vlm, asr and imagegen write `telemetry.v1` NDJSON that
//! scrapes the mock `--telemetry-fixture` page and joins to their request
//! rows on one monotonic clock. llm sends its requests to the Rust mock; vlm,
//! asr and imagegen send theirs to dummy-model-server in strict mode and are
//! skipped when `go` is missing (CI sets `METRUM_BENCH_REQUIRE_DUMMY=1`).

mod common;

use common::{request_records, sine_wav, skip, spawn_dummy, spawn_mock, summary_record, tiny_png};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const REQUESTS: u64 = 4;
const WARMUP: u64 = 1;

/// Two sources on the fixture page, scraped every 100 ms.
fn telemetry_yaml(dir: &Path, metrics: SocketAddr) -> PathBuf {
    let path = dir.join("telemetry.yaml");
    std::fs::write(
        &path,
        format!(
            r#"
default_interval_ms: 1000
timeout_ms: 500
sources:
  - name: all-smi
    url: http://{metrics}/metrics
    interval_ms: 100
    include:
      - "^all_smi_(gpu|cpu|memory)_"
  - name: dcgm
    url: http://{metrics}/metrics
    interval_ms: 100
    include:
      - "^DCGM_FI_DEV_(POWER_USAGE|GPU_UTIL)$"
"#
        ),
    )
    .expect("write telemetry yaml");
    path
}

struct Run {
    _dir: tempfile::TempDir,
    data_log: PathBuf,
    ndjson: PathBuf,
}

fn telemetry_flags(run: &Run, yaml: &Path) -> Vec<String> {
    [
        "--ndjson",
        run.ndjson.to_str().unwrap(),
        "--telemetry",
        yaml.to_str().unwrap(),
        "--require-telemetry",
        "--warmup-requests",
        &WARMUP.to_string(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn new_run() -> Run {
    let dir = tempfile::tempdir().expect("tmpdir");
    Run {
        data_log: dir.path().join("out.jsonl"),
        ndjson: dir.path().join("run.ndjson"),
        _dir: dir,
    }
}

fn logs(run: &Run) -> Vec<String> {
    let dir = run._dir.path();
    [
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

/// Shared acceptance: every kind present, request rows join the data log on
/// `seq` with matching timing, and telemetry samples sit inside the
/// measured stage window on the same clock.
fn assert_joined(run: &Run, binary: &str) {
    let rows = ndjson_rows(&run.ndjson);
    let mut kinds = BTreeMap::<String, u64>::new();
    for row in &rows {
        *kinds
            .entry(row["kind"].as_str().expect("kind").to_string())
            .or_default() += 1;
    }
    assert_eq!(rows[0]["kind"], "run", "run row first: {kinds:?}");
    assert_eq!(
        rows[0]["schema_version"],
        "metrum-ai-bench-cli.telemetry.v1"
    );
    assert_eq!(rows[0]["config"]["binary"], binary);
    assert_eq!(
        rows[0]["telemetry_sources"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(kinds.get("request").copied(), Some(REQUESTS), "{kinds:?}");
    assert_eq!(kinds.get("stage").copied(), Some(2), "{kinds:?}");
    assert!(
        kinds.get("telemetry").copied().unwrap_or(0) >= 4,
        "expected telemetry samples, kinds={kinds:?}"
    );
    assert_eq!(kinds.get("scrape_error"), None, "{kinds:?}");
    let summary = rows.last().expect("rows");
    assert_eq!(summary["kind"], "summary");
    assert_eq!(summary["partial"], false);
    assert_eq!(summary["dropped_telemetry_rows"], 0);
    assert_eq!(summary["request_rows"], REQUESTS);
    assert_eq!(summary["stage_rows"], 2);
    assert_eq!(summary["telemetry_rows"], kinds["telemetry"]);
    let run_id = rows[0]["run_id"].as_str().expect("run_id");
    assert!(rows.iter().all(|row| row["run_id"] == run_id));

    // Request rows join request.v3 records on seq, with the same timing.
    let records: BTreeMap<u64, Value> = request_records(&run.data_log)
        .into_iter()
        .map(|r| (r["seq"].as_u64().expect("seq"), r))
        .collect();
    assert_eq!(records.len() as u64, REQUESTS);
    let requests: Vec<&Value> = rows.iter().filter(|r| r["kind"] == "request").collect();
    for row in &requests {
        let seq = row["seq"].as_u64().expect("seq");
        let record = &records[&seq];
        assert_eq!(record["run_id"], run_id);
        assert_eq!(row["warmup"], record["phase"] == "warmup");
        assert_eq!(row["success"], true, "request row {row}");
        let sent = row["t_sent_ns"].as_u64().expect("t_sent_ns");
        let done = row["t_done_ns"].as_u64().expect("t_done_ns");
        let latency_ns = record["latency_s"].as_f64().expect("latency_s") * 1e9;
        assert!(
            (done as f64 - sent as f64 - latency_ns).abs() < 1_000.0,
            "t_done - t_sent must equal latency_s: {row} vs {record}"
        );
        let offset_ns = record["send_offset_s"].as_f64().expect("send_offset_s") * 1e9;
        assert!(sent as f64 >= offset_ns - 1_000.0, "base offset >= 0");
    }

    // Stage windows bracket their requests, and telemetry lands inside the
    // measured window on the same epoch.
    let measure = rows
        .iter()
        .find(|r| r["kind"] == "stage" && r["phase"] == "measure")
        .expect("measure stage");
    let (start, end) = (
        measure["t_start_ns"].as_u64().unwrap(),
        measure["t_end_ns"].as_u64().unwrap(),
    );
    for row in requests.iter().filter(|r| r["warmup"] == false) {
        assert!(row["t_sent_ns"].as_u64().unwrap() >= start);
        assert!(row["t_done_ns"].as_u64().unwrap() <= end);
    }
    let inside = rows
        .iter()
        .filter(|r| r["kind"] == "telemetry")
        .filter(|r| (start..=end).contains(&r["t_ns"].as_u64().unwrap()))
        .count();
    assert!(inside > 0, "no telemetry sample inside measure window");

    // summary.v3 carries the additive telemetry stamp.
    let run_summary = summary_record(&run.data_log).expect("summary.v3");
    let stamp = &run_summary["telemetry"];
    assert_eq!(stamp["schema_version"], "metrum-ai-bench-cli.telemetry.v1");
    assert_eq!(stamp["ndjson"], run.ndjson.to_str().unwrap());
    assert_eq!(stamp["sources"], 2);
    assert_eq!(stamp["request_rows"], REQUESTS);
    assert_eq!(stamp["telemetry_rows"], kinds["telemetry"]);
    assert_eq!(stamp["dropped_telemetry_rows"], 0);
}

fn llm_command(run: &Run, url: &str) -> Command {
    let prompts = run._dir.path().join("prompts.jsonl");
    std::fs::write(&prompts, "{\"prompt\":\"hello telemetry\"}\n").expect("write prompts");
    let mut command = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-llm"));
    command
        .args([
            "--url",
            url,
            "--api-key",
            "dummy",
            "--scenario",
            "e2e-telemetry",
            "--num-requests",
            &REQUESTS.to_string(),
            "--concurrency",
            "1",
            "--prompts",
            prompts.to_str().unwrap(),
            "--mode",
            "chat",
            "--streaming",
            "--model",
            "mock",
            "--max-tokens",
            "8",
            "--data-log",
            run.data_log.to_str().unwrap(),
            "--log-level",
            "error",
        ])
        .args(logs(run));
    command
}

#[test]
fn llm_telemetry_joins_request_rows() {
    let mock = spawn_mock(&["--latency-ms", "100", "--telemetry-fixture"]);
    let run = new_run();
    let yaml = telemetry_yaml(run._dir.path(), mock.address);
    let output = llm_command(
        &run,
        &format!("http://{}/v1/chat/completions", mock.address),
    )
    .args(telemetry_flags(&run, &yaml))
    .output()
    .expect("run llm");
    assert_ok(&output, "llm");
    assert_joined(&run, "metrum-ai-bench-cli-llm");
}

#[test]
fn vlm_telemetry_joins_request_rows() {
    let mock = spawn_mock(&["--telemetry-fixture"]);
    let Some(dummy) = spawn_dummy(&["-latency", "100ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let run = new_run();
    let yaml = telemetry_yaml(run._dir.path(), mock.address);
    let image = run._dir.path().join("pixel.png");
    std::fs::write(&image, tiny_png()).expect("write png");
    let prompts = run._dir.path().join("prompts.jsonl");
    let mut file = std::fs::File::create(&prompts).expect("prompts");
    writeln!(
        file,
        r#"{{"prompt":"Describe this image","image_url":"{}"}}"#,
        image.to_str().unwrap()
    )
    .expect("write prompts");
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-vlm"))
        .args([
            "--url",
            &dummy.url("/v1/chat/completions"),
            "--api-key",
            "dummy",
            "--scenario",
            "e2e-telemetry",
            "--num-requests",
            &REQUESTS.to_string(),
            "--concurrency",
            "1",
            "--prompts",
            prompts.to_str().unwrap(),
            "--model",
            "dummy",
            "--max-tokens",
            "8",
            "--data-log",
            run.data_log.to_str().unwrap(),
            "--log-level",
            "error",
        ])
        .args(logs(&run))
        .args(telemetry_flags(&run, &yaml))
        .output()
        .expect("run vlm");
    assert_ok(&output, "vlm");
    assert_joined(&run, "metrum-ai-bench-cli-vlm");
}

#[test]
fn asr_telemetry_joins_request_rows() {
    let mock = spawn_mock(&["--telemetry-fixture"]);
    let Some(dummy) = spawn_dummy(&["-latency", "100ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let run = new_run();
    let yaml = telemetry_yaml(run._dir.path(), mock.address);
    let audio = run._dir.path().join("sample.wav");
    std::fs::write(&audio, sine_wav(1.0)).expect("write wav");
    let input = run._dir.path().join("input.jsonl");
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
            "--api-key",
            "dummy",
            "--scenario",
            "e2e-telemetry",
            "--num-requests",
            &REQUESTS.to_string(),
            "--concurrency",
            "1",
            "--input",
            input.to_str().unwrap(),
            "--model",
            "dummy",
            "--data-log",
            run.data_log.to_str().unwrap(),
            "--log-level",
            "error",
        ])
        .args(logs(&run))
        .args(telemetry_flags(&run, &yaml))
        .output()
        .expect("run asr");
    assert_ok(&output, "asr");
    assert_joined(&run, "metrum-ai-bench-cli-asr");
}

#[test]
fn imagegen_telemetry_joins_request_rows() {
    let mock = spawn_mock(&["--telemetry-fixture"]);
    let Some(dummy) = spawn_dummy(&["-latency", "100ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let run = new_run();
    let yaml = telemetry_yaml(run._dir.path(), mock.address);
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-imagegen"))
        .args([
            "--url",
            &dummy.url("/v1"),
            "--api-key",
            "dummy",
            "--scenario",
            "e2e-telemetry",
            "--model",
            "dummy",
            "--num-requests",
            &REQUESTS.to_string(),
            "--concurrency",
            "1",
            "--prompt",
            "a small test image",
            "--size",
            "64x64",
            "--data-log",
            run.data_log.to_str().unwrap(),
            "--artifact-dir",
            run._dir.path().join("artifacts").to_str().unwrap(),
            "--no-save-images",
        ])
        .args(logs(&run))
        .args(telemetry_flags(&run, &yaml))
        .output()
        .expect("run imagegen");
    assert_ok(&output, "imagegen");
    assert_joined(&run, "metrum-ai-bench-cli-imagegen");
}

/// A source nobody serves fails the run before any request is sent.
#[test]
fn llm_require_telemetry_fails_when_no_source_scrapes() {
    let mock = spawn_mock(&["--latency-ms", "5"]);
    let run = new_run();
    // Bind and drop a listener so the port is free and refuses connections.
    let dead = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr");
    let yaml = telemetry_yaml(run._dir.path(), dead);
    let output = llm_command(
        &run,
        &format!("http://{}/v1/chat/completions", mock.address),
    )
    .args(telemetry_flags(&run, &yaml))
    .output()
    .expect("run llm");
    assert!(!output.status.success(), "dead telemetry source must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("could not be scraped at startup"),
        "stderr={stderr}"
    );
    assert!(
        request_records(&run.data_log).is_empty(),
        "no request may be sent when telemetry cannot start"
    );
}

#[test]
fn require_telemetry_without_yaml_is_rejected() {
    let run = new_run();
    let output = llm_command(&run, "http://127.0.0.1:9/v1/chat/completions")
        .args([
            "--ndjson",
            run.ndjson.to_str().unwrap(),
            "--require-telemetry",
        ])
        .output()
        .expect("run llm");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--require-telemetry needs --telemetry"),
        "stderr={stderr}"
    );
}

/// `--ndjson` alone still writes run, request, stage and summary rows.
#[test]
fn llm_ndjson_without_sources_writes_request_rows() {
    let mock = spawn_mock(&["--latency-ms", "5"]);
    let run = new_run();
    let output = llm_command(
        &run,
        &format!("http://{}/v1/chat/completions", mock.address),
    )
    .args(["--ndjson", run.ndjson.to_str().unwrap()])
    .output()
    .expect("run llm");
    assert_ok(&output, "llm");
    let rows = ndjson_rows(&run.ndjson);
    let kinds: Vec<&str> = rows.iter().map(|r| r["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds.first(), Some(&"run"));
    assert_eq!(kinds.last(), Some(&"summary"));
    assert_eq!(
        kinds.iter().filter(|k| **k == "request").count() as u64,
        REQUESTS
    );
    assert!(!kinds.contains(&"telemetry"));
    let run_summary = summary_record(&run.data_log).expect("summary.v3");
    assert_eq!(run_summary["telemetry"]["sources"], 0);
}
