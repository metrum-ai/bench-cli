// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! E2E (#196): llm, vlm, asr and imagegen write `telemetry.v1` NDJSON that
//! scrapes the mock `--telemetry-fixture` page and joins to their request
//! rows on one monotonic clock. The joins send their requests to
//! dummy-model-server in strict mode and are skipped when `go` is missing
//! (CI sets `METRUM_BENCH_REQUIRE_DUMMY=1`); flag checks use the Rust mock.

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
/// Allowed gap between `run.t0_wall + t_sent_ns` and `started_at`. The
/// wall/monotonic pairs are sampled separately and `t0_wall` is truncated to
/// the millisecond, so allow a little preemption on a busy CI host. This
/// check catches a wrong origin only: `send_offset_s` and `started_at` are
/// stamped together, so a file read inside the request window cannot show
/// up here (`asr_request_window_excludes_audio_read` covers that).
const WALL_JOIN_TOLERANCE_MS: f64 = 15.0;

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
    let t0_wall =
        chrono::DateTime::parse_from_rfc3339(rows[0]["t0_wall"].as_str().expect("t0_wall"))
            .expect("t0_wall rfc3339")
            .with_timezone(&chrono::Utc);

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
        // One origin: the data-log offset is the NDJSON send time (1 us
        // tolerance for the f64 seconds round trip).
        let offset_ns = record["send_offset_s"].as_f64().expect("send_offset_s") * 1e9;
        assert!(
            (sent as f64 - offset_ns).abs() < 1_000.0,
            "send_offset_s * 1e9 must equal t_sent_ns: {row} vs {record}"
        );
        // Wall-clock join (#227): run.t0_wall + t_sent_ns lands on the
        // record's started_at, which catches a wrong origin (not a read
        // inside the window). See WALL_JOIN_TOLERANCE_MS.
        let started_at = chrono::DateTime::parse_from_rfc3339(
            record["started_at"].as_str().expect("started_at"),
        )
        .expect("started_at rfc3339");
        let sent_wall = t0_wall + chrono::Duration::nanoseconds(sent as i64);
        let skew_ms = (started_at.with_timezone(&chrono::Utc) - sent_wall)
            .num_microseconds()
            .expect("skew") as f64
            / 1e3;
        assert!(
            skew_ms.abs() < WALL_JOIN_TOLERANCE_MS,
            "t0_wall + t_sent_ns is {skew_ms} ms from started_at: {row} vs {record}"
        );
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
    assert_eq!(
        stamp["ndjson"], "run.ndjson",
        "file name only, no local dirs"
    );
    assert_eq!(stamp["sources"], 2);
    assert_eq!(stamp["request_rows"], REQUESTS);
    assert_eq!(stamp["telemetry_rows"], kinds["telemetry"]);
    assert_eq!(stamp["dropped_telemetry_rows"], 0);
}

fn llm_command(run: &Run, url: &str) -> Command {
    llm_command_at(run, url, "error")
}

fn llm_command_at(run: &Run, url: &str, log_level: &str) -> Command {
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
            log_level,
        ])
        .args(logs(run));
    command
}

#[test]
fn llm_telemetry_joins_request_rows() {
    let mock = spawn_mock(&["--telemetry-fixture"]);
    let Some(dummy) = spawn_dummy(&["-latency", "100ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let run = new_run();
    let yaml = telemetry_yaml(run._dir.path(), mock.address);
    let output = llm_command(&run, &dummy.url("/v1/chat/completions"))
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

/// A source nobody serves fails the run before any request is sent, with
/// and without `--require-telemetry`.
#[test]
fn llm_require_telemetry_fails_when_no_source_scrapes() {
    dead_source_fails_before_requests(true);
}

#[test]
fn llm_dead_source_fails_without_require_telemetry() {
    dead_source_fails_before_requests(false);
}

fn dead_source_fails_before_requests(require: bool) {
    let mock = spawn_mock(&["--latency-ms", "5"]);
    let run = new_run();
    // Bind and drop a listener so the port is free and refuses connections.
    let dead = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr");
    let yaml = telemetry_yaml(run._dir.path(), dead);
    let mut flags = telemetry_flags(&run, &yaml);
    if !require {
        flags.retain(|flag| flag != "--require-telemetry");
    }
    let output = llm_command(
        &run,
        &format!("http://{}/v1/chat/completions", mock.address),
    )
    .args(flags)
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

/// #227: a Ctrl-C after a mid-run `--require-telemetry` abort drains like a
/// first signal. Before the fix the abort set the flag the signal handler
/// checks, so this SIGINT hard-exited (130) before `summary.v3` and the
/// NDJSON summary row were written.
#[cfg(unix)]
#[test]
fn llm_sigint_after_telemetry_abort_still_writes_summaries() {
    use std::io::BufRead;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    // Slow requests keep the drain open long enough to signal into it.
    let mock = spawn_mock(&["--latency-ms", "3000"]);
    let source = common::spawn_flaky_metrics(1);
    let run = new_run();
    let yaml = common::flaky_telemetry_yaml(run._dir.path(), source);
    // warn level, so the stop handler's "Ctrl-C received" line is logged.
    let mut command = llm_command_at(
        &run,
        &format!("http://{}/v1/chat/completions", mock.address),
        "warn",
    );
    let mut child = command
        .args([
            "--ndjson",
            run.ndjson.to_str().unwrap(),
            "--telemetry",
            yaml.to_str().unwrap(),
            "--require-telemetry",
            "--require-telemetry-failures",
            "2",
        ])
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn llm");
    let stderr = child.stderr.take().expect("stderr");
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stderr)
            .lines()
            .map_while(Result::ok)
        {
            let _ = tx.send(line);
        }
    });
    let mut lines = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(20);
    let aborted = loop {
        let Ok(line) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) else {
            break false;
        };
        let hit = line.contains("(require-telemetry)") && line.contains("stopping new requests");
        lines.push(line);
        if hit {
            break true;
        }
    };
    if !aborted {
        let _ = child.kill();
        panic!("no telemetry abort logged: {lines:#?}");
    }
    // The abort fired; the first request (3 s, concurrency 1) is in flight.
    if child.try_wait().expect("poll llm").is_some() {
        panic!("llm exited before the signal: {lines:#?}");
    }
    let status = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status();
    if !status.as_ref().is_ok_and(|status| status.success()) {
        let _ = child.kill();
        panic!("could not send SIGINT: {status:?}");
    }
    let exit = child.wait().expect("wait llm");
    lines.extend(rx.try_iter());
    let stderr = lines.join("\n");
    assert_ne!(
        exit.code(),
        Some(130),
        "hard exit on first signal: {stderr}"
    );
    assert!(!exit.success(), "a telemetry abort fails the run: {stderr}");
    // The handler saw the signal as a first Ctrl-C and drained (warn goes
    // to stdout in simplelog's mixed mode, so read the debug log).
    let debug_log = std::fs::read_to_string(run._dir.path().join("debug.log")).expect("debug log");
    assert!(debug_log.contains("Ctrl-C received"), "{debug_log}");
    assert!(!debug_log.contains("Second Ctrl-C"), "{debug_log}");
    let run_summary = summary_record(&run.data_log).expect("summary.v3 written");
    assert!(run_summary["telemetry"].is_object(), "{run_summary}");
    let rows = ndjson_rows(&run.ndjson);
    let last = rows.last().expect("ndjson rows");
    assert_eq!(last["kind"], "summary", "{last}");
    assert_eq!(last["partial"], true, "{last}");
    assert!(
        last["request_rows"].as_u64().unwrap_or(0) < REQUESTS,
        "the abort stops new requests: {last}"
    );
}

/// #227: the ASR request window starts after the audio file read. The audio
/// is a FIFO whose writer stalls for `STALL` once ASR opens it, so a read
/// inside the window would add the stall to `latency_s` and leave
/// `send_offset_s` before it.
#[cfg(unix)]
#[test]
fn asr_request_window_excludes_audio_read() {
    use std::time::Duration;
    const STALL: Duration = Duration::from_millis(1000);
    let Some(dummy) = spawn_dummy(&["-latency", "100ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let run = new_run();
    let audio = run._dir.path().join("sample.wav");
    let status = Command::new("mkfifo")
        .arg(&audio)
        .status()
        .expect("run mkfifo");
    assert!(status.success(), "mkfifo failed");
    let input = run._dir.path().join("input.jsonl");
    std::fs::write(
        &input,
        format!(
            "{{\"id\":\"sample-1\",\"path\":\"{}\",\"format\":\"wav\",\"duration\":1.0}}\n",
            audio.to_str().unwrap()
        ),
    )
    .expect("write input");
    let fifo = audio.clone();
    // Opening for write blocks until ASR opens the FIFO to read it.
    let writer = std::thread::spawn(move || {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(&fifo)
            .expect("open fifo");
        std::thread::sleep(STALL);
        file.write_all(&sine_wav(1.0)).expect("write wav");
    });
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-asr"))
        .args([
            "--url",
            &dummy.url("/v1/audio/transcriptions"),
            "--api-key",
            "dummy",
            "--scenario",
            "e2e-asr-read",
            "--num-requests",
            "1",
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
        .output()
        .expect("run asr");
    // If ASR never opened the FIFO, the writer is still blocked in open:
    // open the read end once to release it instead of hanging the test.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !writer.is_finished() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    if !writer.is_finished() {
        let _ = std::fs::File::open(&audio);
    }
    assert_ok(&output, "asr");
    writer.join().expect("fifo writer");
    let records = request_records(&run.data_log);
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert!(record["error"].is_null(), "{record}");
    let latency = record["latency_s"].as_f64().expect("latency_s");
    let send_offset = record["send_offset_s"].as_f64().expect("send_offset_s");
    let stall = STALL.as_secs_f64();
    assert!(
        latency < stall / 2.0,
        "latency_s {latency} includes the {stall} s file read: {record}"
    );
    assert!(
        send_offset >= stall * 0.9,
        "send_offset_s {send_offset} was taken before the {stall} s file read: {record}"
    );
}

/// #227: a sample whose audio cannot be read fails that request on the
/// client side: a non-connect error, with the send offset and latency taken
/// after the failed read (no file read inside the window).
#[test]
fn asr_unreadable_audio_fails_request_without_connect_error() {
    let mock = spawn_mock(&["--latency-ms", "5"]);
    let run = new_run();
    // A directory passes the startup existence check, then fails the read.
    let audio = run._dir.path().join("not-a-file.wav");
    std::fs::create_dir(&audio).expect("mkdir");
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
            &format!("http://{}/v1/audio/transcriptions", mock.address),
            "--api-key",
            "dummy",
            "--scenario",
            "e2e-asr-unreadable",
            "--num-requests",
            "1",
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
        .output()
        .expect("run asr");
    let records = request_records(&run.data_log);
    assert_eq!(
        records.len(),
        1,
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let record = &records[0];
    let error = &record["error"];
    assert!(!error.is_null(), "{record}");
    let kind = error["kind"].as_str().expect("error kind");
    assert_ne!(kind, "connect", "{record}");
    let latency = record["latency_s"].as_f64().expect("latency_s");
    assert!(latency < 0.05, "latency_s {latency}: {record}");
    assert!(record["send_offset_s"].as_f64().is_some(), "{record}");
}
