// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! #195 end-to-end checks for the time-weighted summary blocks against
//! dummy-model-server in `-strict-media` mode (Metrum AI Bench). Every block
//! average is recomputed from the request rows on disk: each success spans
//! `[send, send + latency_s]` and splits at its first generated token, and
//! the run window holds every success, so integrals need no clipping.
//! Skipped if `go` is missing.

mod common;

use common::{request_records, skip, spawn_dummy, summary_record};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

const MAX_TOKENS: u64 = 8;
const REQUESTS: usize = 8;

fn run_llm(url: &str, dir: &Path, streaming: bool) -> PathBuf {
    let prompts = dir.join("prompts.jsonl");
    std::fs::write(&prompts, "{\"prompt\":\"List four colors.\"}\n").expect("prompts");
    let data_log = dir.join(if streaming {
        "stream.jsonl"
    } else {
        "unary.jsonl"
    });
    let mut command = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-llm"));
    command.args([
        "--url",
        url,
        "--api-key",
        "dummy",
        "--scenario",
        "time-weighted",
        "--num-requests",
        &REQUESTS.to_string(),
        "--concurrency",
        "4",
        "--prompts",
        prompts.to_str().expect("utf8"),
        "--mode",
        "chat",
        "--model",
        "dummy",
        "--max-tokens",
        &MAX_TOKENS.to_string(),
        "--data-log",
        data_log.to_str().expect("utf8"),
        "--debug-log",
        dir.join("debug.log").to_str().expect("utf8"),
        "--error-log",
        dir.join("error.log").to_str().expect("utf8"),
        "--log-level",
        "error",
    ]);
    if streaming {
        command.arg("--streaming");
    }
    let status = command.status().expect("run llm");
    assert!(status.success(), "llm bench failed");
    data_log
}

fn f64_at(value: &Value, key: &str) -> f64 {
    value[key]
        .as_f64()
        .unwrap_or_else(|| panic!("{key}: {value}"))
}

fn assert_close(actual: &Value, expected: f64, label: &str) {
    let actual = actual
        .as_f64()
        .unwrap_or_else(|| panic!("{label}: {actual}"));
    assert!(
        (actual - expected).abs() <= 1e-9 * expected.abs().max(1.0),
        "{label}: {actual} != {expected}"
    );
}

/// Prefill end per row: the first generated token of any kind.
fn first_token_s(record: &Value) -> f64 {
    let ttft = f64_at(record, "ttft_s");
    record["first_reasoning_s"]
        .as_f64()
        .map_or(ttft, |reasoning| reasoning.min(ttft))
}

#[test]
fn llm_time_weighted_blocks_match_request_rows() {
    let Some(dummy) = spawn_dummy(&["-latency", "20ms", "-chunk-interval", "5ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let dir = tempfile::tempdir().expect("tmpdir");
    let url = dummy.url("/v1/chat/completions");

    let data_log = run_llm(&url, dir.path(), true);
    let records = request_records(&data_log);
    assert_eq!(records.len(), REQUESTS);
    let summary = summary_record(&data_log).expect("summary");
    let window = f64_at(&summary, "window_seconds");
    let mut in_flight = 0.0;
    let mut prefill = 0.0;
    let mut decode = 0.0;
    let mut tokens = 0.0;
    let mut input = 0.0;
    let mut output = 0.0;
    for record in &records {
        let latency = f64_at(record, "latency_s");
        let first = first_token_s(record);
        let isl = f64_at(record, "prompt_tokens");
        let osl = f64_at(record, "completion_tokens");
        in_flight += latency;
        prefill += first;
        decode += latency - first;
        tokens += isl * latency + osl * (latency - first) / 2.0;
        input += isl;
        output += osl;
    }
    assert_eq!(summary["effective_concurrency"]["n"], REQUESTS);
    assert_close(
        &summary["effective_concurrency"]["avg"],
        in_flight / window,
        "concurrency",
    );
    let max = f64_at(&summary["effective_concurrency"], "max");
    assert!((1.0..=4.0).contains(&max), "max {max} within the cap");
    assert_close(
        &summary["effective_prefill_concurrency"]["avg"],
        prefill / window,
        "prefill concurrency",
    );
    assert_close(
        &summary["effective_decode_concurrency"]["avg"],
        decode / window,
        "decode concurrency",
    );
    assert_close(
        &summary["tokens_in_flight"]["avg"],
        tokens / window,
        "tokens",
    );
    assert_close(
        &summary["effective_prefill_throughput"]["avg"],
        input / window,
        "prefill tok/s",
    );
    assert_close(
        &summary["effective_decode_throughput"]["avg"],
        output / window,
        "decode tok/s",
    );

    // Unary: in flight, but no first token, so the phase blocks are n = 0 / null.
    let unary = summary_record(&run_llm(&url, dir.path(), false)).expect("summary");
    assert_eq!(unary["effective_concurrency"]["n"], REQUESTS);
    for key in [
        "effective_prefill_concurrency",
        "effective_decode_concurrency",
        "tokens_in_flight",
        "effective_prefill_throughput",
        "effective_decode_throughput",
    ] {
        assert_eq!(unary[key]["n"], 0, "{key}");
        assert!(unary[key]["avg"].is_null(), "{key}");
    }
}

#[test]
fn strategic_points_report_time_weighted_blocks() {
    let Some(dummy) = spawn_dummy(&["-latency", "20ms", "-chunk-interval", "5ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let dir = tempfile::tempdir().expect("tmpdir");
    let prompts = dir.path().join("prompts.jsonl");
    std::fs::write(&prompts, "{\"prompt\":\"alpha\"}\n").expect("prompts");
    let csv = dir.path().join("records.csv");
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"))
        .args([
            "--url",
            &dummy.url("/v1/chat/completions"),
            "--model",
            "dummy",
            "--prompts",
            prompts.to_str().expect("utf8"),
            "--max-tokens",
            &MAX_TOKENS.to_string(),
            "--sweep",
            "2",
            "--requests-per-stage",
            "4",
            "--streaming",
        ])
        .arg("--csv")
        .arg(&csv)
        .arg("--html")
        .arg(dir.path().join("report.html"))
        .output()
        .expect("run strategic");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let summary: Value = serde_json::from_slice(&output.stdout).expect("summary");
    let point = &summary["points"][0];
    for key in [
        "effective_concurrency",
        "effective_prefill_concurrency",
        "effective_decode_concurrency",
        "tokens_in_flight",
        "effective_prefill_throughput",
        "effective_decode_throughput",
    ] {
        assert_eq!(point[key]["n"], 4, "{key}");
        let avg = f64_at(&point[key], "avg");
        assert!(avg > 0.0, "{key}: {avg}");
    }
    let max = f64_at(&point["effective_concurrency"], "max");
    assert!((1.0..=2.0).contains(&max), "max {max} within the stage cap");
    // The time-weighted window holds every success, so in-flight time and
    // output tokens recompute exactly from the request CSV rows.
    let mut reader = csv::Reader::from_path(&csv).expect("csv");
    let headers = reader.headers().expect("headers").clone();
    let col = |row: &csv::StringRecord, name: &str| -> String {
        let index = headers.iter().position(|h| h == name).expect(name);
        row[index].to_string()
    };
    let rows: Vec<csv::StringRecord> = reader.records().map(|row| row.expect("row")).collect();
    let stage = |row: &csv::StringRecord| col(row, "stage").parse::<f64>().expect("stage");
    let measured: Vec<&csv::StringRecord> = rows
        .iter()
        .filter(|row| col(row, "warmup") == "false" && stage(row) == f64_at(point, "load"))
        .collect();
    let sent = |row: &csv::StringRecord| col(row, "sent_unix_ns").parse::<u128>().expect("sent");
    let service = |row: &csv::StringRecord| {
        col(row, "service_latency_s")
            .parse::<f64>()
            .expect("service")
    };
    let first = measured.iter().map(|row| sent(row)).min().expect("rows");
    let ok: Vec<&&csv::StringRecord> = measured
        .iter()
        .filter(|row| col(row, "success") == "true")
        .collect();
    assert_eq!(ok.len(), 4);
    let window = ok
        .iter()
        .map(|row| (sent(row) - first) as f64 / 1e9 + service(row))
        .fold(0.0, f64::max);
    let in_flight: f64 = ok.iter().map(|row| service(row)).sum();
    let output: f64 = ok
        .iter()
        .map(|row| col(row, "output_tokens").parse::<f64>().expect("output"))
        .sum();
    assert_close(
        &point["effective_concurrency"]["avg"],
        in_flight / window,
        "concurrency",
    );
    assert_close(
        &point["effective_decode_throughput"]["avg"],
        output / window,
        "decode tok/s",
    );
}
