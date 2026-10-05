// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! #194 end-to-end checks for the HTTP phase trace against dummy-model-server
//! in `-strict-media` mode (Metrum AI Bench): connection reuse on pooled
//! connections, DNS time on a hostname URL, body bytes, chunks, and receive
//! time, plus the matching summary and strategic sweep-point aggregates.
//! Skipped if `go` is missing.

mod common;

use common::{request_records, skip, spawn_dummy, summary_record};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

const MAX_TOKENS: u64 = 8;
const REQUESTS: usize = 4;
/// Dummy pacing between stream chunks.
const CHUNK_INTERVAL_S: f64 = 0.010;

fn run_llm(url: &str, dir: &Path, streaming: bool) -> PathBuf {
    let prompts = dir.join("prompts.jsonl");
    std::fs::write(&prompts, "{\"prompt\":\"Name three rivers.\"}\n").expect("prompts");
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
        "http-trace",
        "--num-requests",
        &REQUESTS.to_string(),
        "--concurrency",
        "1",
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

fn u64_at(value: &Value, key: &str) -> u64 {
    value[key]
        .as_u64()
        .unwrap_or_else(|| panic!("{key}: {value}"))
}

/// Per-request trace checks shared by streaming and unary runs. Requests run
/// one at a time, so only the first opens a connection; the rest reuse it.
fn assert_trace(data_log: &Path, streaming: bool, hostname: bool) {
    let label = if streaming { "streaming" } else { "unary" };
    let mut records = request_records(data_log);
    records.sort_by_key(|r| r["seq"].as_u64());
    assert_eq!(records.len(), REQUESTS, "{label}");
    for (index, record) in records.iter().enumerate() {
        let reused = record["connection_reused"]
            .as_bool()
            .unwrap_or_else(|| panic!("{label}: connection_reused {record}"));
        let connect = f64_at(record, "connect_s");
        let dns = f64_at(record, "dns_s");
        if index == 0 {
            assert!(!reused, "{label}: first request opens a connection");
            assert!(connect > 0.0, "{label}: fresh connect time {connect}");
            if hostname {
                assert!(dns > 0.0, "{label}: hostname lookup time {dns}");
                assert!(
                    dns <= connect,
                    "{label}: dns {dns} inside connect {connect}"
                );
            } else {
                assert_eq!(dns, 0.0, "{label}: IP literal skips DNS");
            }
        } else {
            assert!(
                reused,
                "{label}: request {index} rides the pooled connection"
            );
            assert_eq!(connect, 0.0, "{label}: pool hit has no connect time");
            assert_eq!(dns, 0.0, "{label}: pool hit has no lookup");
        }
        let sent = u64_at(record, "bytes_sent");
        assert!(sent > 0, "{label}: request body bytes");
        let received = u64_at(record, "bytes_received");
        let chunks = u64_at(record, "chunks_received");
        let receive = f64_at(record, "receive_s");
        let first_byte = f64_at(record, "first_byte_s");
        let latency = f64_at(record, "latency_s");
        assert!(received > 0, "{label}: response body bytes");
        assert!(
            chunks >= 1 && chunks <= received,
            "{label}: chunks {chunks}"
        );
        assert!(
            first_byte + receive <= latency + 1e-3,
            "{label}: headers {first_byte}s + receive {receive}s exceed latency {latency}s"
        );
        if streaming {
            // Wire check: paced SSE events arrive as separate body chunks.
            assert!(chunks >= MAX_TOKENS, "{label}: chunks {chunks}");
            let paced = (MAX_TOKENS - 1) as f64 * CHUNK_INTERVAL_S * 0.5;
            assert!(receive >= paced, "{label}: receive {receive}s < {paced}s");
        }
    }

    let summary = summary_record(data_log).expect("summary");
    assert_eq!(summary["connections_reused"], REQUESTS - 1, "{label}");
    let rate = f64_at(&summary, "connection_reuse_rate");
    let expected = (REQUESTS - 1) as f64 / REQUESTS as f64;
    assert!((rate - expected).abs() < 1e-12, "{label}: rate {rate}");
    for key in [
        "dns_s",
        "receive_s",
        "bytes_sent",
        "bytes_received",
        "chunks_received",
    ] {
        assert_eq!(summary[key]["n"], REQUESTS, "{label}: {key}");
    }
    let mean_received = records
        .iter()
        .map(|r| u64_at(r, "bytes_received") as f64)
        .sum::<f64>()
        / REQUESTS as f64;
    let avg = f64_at(&summary["bytes_received"], "avg");
    assert!((avg - mean_received).abs() < 1e-9, "{label}: avg {avg}");
}

#[test]
fn llm_records_http_phase_trace_and_pool_reuse() {
    let Some(dummy) = spawn_dummy(&["-latency", "20ms", "-chunk-interval", "10ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let dir = tempfile::tempdir().expect("tmpdir");
    let ip_url = dummy.url("/v1/chat/completions");
    assert_trace(&run_llm(&ip_url, dir.path(), true), true, false);
    assert_trace(&run_llm(&ip_url, dir.path(), false), false, false);
    let host_dir = tempfile::tempdir().expect("tmpdir");
    let host_url = ip_url.replacen("127.0.0.1", "localhost", 1);
    assert_trace(&run_llm(&host_url, host_dir.path(), true), true, true);
}

#[test]
fn strategic_points_report_http_phase_trace() {
    let Some(dummy) = spawn_dummy(&["-latency", "20ms", "-chunk-interval", "10ms"]) else {
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
            "1",
            "--requests-per-stage",
            "3",
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
        "dns_s",
        "receive_s",
        "bytes_sent",
        "bytes_received",
        "chunks_received",
    ] {
        assert_eq!(point[key]["n"], 3, "{key}");
    }

    let mut reader = csv::Reader::from_path(&csv).expect("csv");
    let headers = reader.headers().expect("headers").clone();
    let col = |row: &csv::StringRecord, name: &str| -> String {
        let index = headers.iter().position(|h| h == name).expect(name);
        row[index].to_string()
    };
    let rows: Vec<csv::StringRecord> = reader.records().map(|row| row.expect("row")).collect();
    let measured: Vec<&csv::StringRecord> = rows
        .iter()
        .filter(|row| col(row, "success") == "true" && col(row, "warmup") == "false")
        .collect();
    assert_eq!(measured.len(), 3);
    let reused = measured
        .iter()
        .filter(|row| col(row, "connection_reused") == "true")
        .count();
    assert_eq!(point["connections_reused"], reused);
    // Concurrency 1 on one pooled connection: at most the first row connects.
    assert!(reused >= 2, "reused {reused}");
    for row in &measured {
        let chunks: u64 = col(row, "chunks_received").parse().expect("chunks");
        assert!(chunks >= MAX_TOKENS, "chunks {chunks}");
        let received: u64 = col(row, "bytes_received").parse().expect("bytes");
        assert!(received > 0);
        let sent: u64 = col(row, "bytes_sent").parse().expect("bytes_sent");
        assert!(sent > 0);
        let receive: f64 = col(row, "receive_s").parse().expect("receive_s");
        assert!(receive > 0.0);
    }
}
