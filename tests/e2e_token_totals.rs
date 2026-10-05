// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! #193 end-to-end checks for run token totals and per-user rates against
//! dummy-model-server in `-strict-media` mode (Metrum AI Bench). Every
//! summary value is recomputed from the request rows on disk. Skipped if
//! `go` is missing.

mod common;

use common::{request_records, skip, spawn_dummy, summary_record};
use serde_json::Value;
use std::path::Path;
use std::process::Command;

const MAX_TOKENS: u64 = 8;
const REQUESTS: usize = 3;
/// Dummy pacing between stream chunks; the first ITL cannot be shorter.
const CHUNK_INTERVAL_S: f64 = 0.010;

fn run_llm(url: &str, dir: &Path, streaming: bool) -> std::path::PathBuf {
    let prompts = dir.join("prompts.jsonl");
    std::fs::write(&prompts, "{\"prompt\":\"Count the primes below fifty.\"}\n").expect("prompts");
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
        "token-totals",
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

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
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

/// Prefill end per row: the first generated token of any kind (reasoning or
/// visible), matching `prefill_tps_per_user`.
fn first_generated_token_s(record: &Value) -> f64 {
    let ttft = f64_at(record, "ttft_s");
    record["first_reasoning_s"]
        .as_f64()
        .map_or(ttft, |reasoning| reasoning.min(ttft))
}

/// Recompute totals and rates from request rows and compare to the summary.
fn assert_matches_rows(data_log: &Path, streaming: bool) {
    let label = if streaming { "streaming" } else { "unary" };
    let records = request_records(data_log);
    assert_eq!(records.len(), REQUESTS, "{label}");
    let summary = summary_record(data_log).expect("summary");
    let window = f64_at(&summary, "window_seconds");

    let prompt: u64 = records
        .iter()
        .map(|r| r["prompt_tokens"].as_u64().expect("prompt_tokens"))
        .sum();
    let completion: u64 = records
        .iter()
        .map(|r| r["completion_tokens"].as_u64().expect("completion_tokens"))
        .sum();
    assert!(prompt > 0, "{label}: dummy reports prompt tokens");
    assert_eq!(completion, REQUESTS as u64 * MAX_TOKENS, "{label}");
    assert_eq!(summary["prompt_tokens_total"], prompt, "{label}");
    assert_eq!(summary["completion_tokens_total"], completion, "{label}");
    assert_close(
        &summary["input_tokens_per_second"],
        prompt as f64 / window,
        label,
    );
    assert_close(
        &summary["total_tokens_per_second"],
        (prompt + completion) as f64 / window,
        label,
    );

    let user: Vec<f64> = records
        .iter()
        .map(|r| r["completion_tokens"].as_f64().unwrap() / f64_at(r, "latency_s"))
        .collect();
    assert_eq!(summary["user_tps"]["n"], REQUESTS, "{label}");
    assert_close(&summary["user_tps"]["avg"], mean(&user), label);

    if !streaming {
        // No TTFT without streaming: not applicable, so n = 0.
        assert_eq!(summary["prefill_tps_per_user"]["n"], 0, "{label}");
        assert_eq!(summary["time_to_second_token_s"]["n"], 0, "{label}");
        return;
    }
    let prefill: Vec<f64> = records
        .iter()
        .map(|r| r["prompt_tokens"].as_f64().unwrap() / first_generated_token_s(r))
        .collect();
    assert_eq!(summary["prefill_tps_per_user"]["n"], REQUESTS, "{label}");
    assert_close(
        &summary["prefill_tps_per_user"]["avg"],
        mean(&prefill),
        label,
    );

    let mut second = Vec::new();
    for record in &records {
        let ttft = f64_at(record, "ttft_s");
        let first_itl = record["itl_s"][0].as_f64().expect("first itl");
        // Wire check: the second chunk arrives one paced interval later.
        assert!(first_itl >= CHUNK_INTERVAL_S * 0.5, "first ITL {first_itl}");
        second.push(ttft + first_itl);
    }
    assert_eq!(summary["time_to_second_token_s"]["n"], REQUESTS, "{label}");
    assert_close(
        &summary["time_to_second_token_s"]["avg"],
        mean(&second),
        label,
    );
    assert!(
        f64_at(&summary["time_to_second_token_s"], "avg") > f64_at(&summary["ttft_s"], "avg"),
        "{label}: second token after first"
    );
}

#[test]
fn llm_token_totals_and_rates_match_request_rows() {
    let Some(dummy) = spawn_dummy(&["-latency", "20ms", "-chunk-interval", "10ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let dir = tempfile::tempdir().expect("tmpdir");
    let url = dummy.url("/v1/chat/completions");
    assert_matches_rows(&run_llm(&url, dir.path(), true), true);
    assert_matches_rows(&run_llm(&url, dir.path(), false), false);
}

/// #221 review: a thinking model's prefill rate ends at the first reasoning
/// token, not at visible TTFT, which would fold reasoning time into prefill.
#[test]
fn llm_prefill_rate_ends_at_first_reasoning_token() {
    let Some(dummy) = spawn_dummy(&["-reasoning", "-latency", "20ms", "-chunk-interval", "10ms"])
    else {
        skip("go dummy-model-server not available");
        return;
    };
    let dir = tempfile::tempdir().expect("tmpdir");
    let data_log = run_llm(&dummy.url("/v1/chat/completions"), dir.path(), true);
    let records = request_records(&data_log);
    assert_eq!(records.len(), REQUESTS);
    let summary = summary_record(&data_log).expect("summary");
    let mut by_reasoning = Vec::new();
    let mut by_visible = Vec::new();
    for record in &records {
        let isl = record["prompt_tokens"].as_f64().expect("prompt_tokens");
        let reasoning = f64_at(record, "first_reasoning_s");
        let ttft = f64_at(record, "ttft_s");
        // Wire check: reasoning streams before the first visible chunk.
        assert!(
            reasoning < ttft,
            "reasoning {reasoning}s not before ttft {ttft}s"
        );
        assert_eq!(first_generated_token_s(record), reasoning);
        by_reasoning.push(isl / reasoning);
        by_visible.push(isl / ttft);
    }
    assert_eq!(summary["prefill_tps_per_user"]["n"], REQUESTS);
    assert_close(
        &summary["prefill_tps_per_user"]["avg"],
        mean(&by_reasoning),
        "reasoning prefill",
    );
    assert!(
        f64_at(&summary["prefill_tps_per_user"], "avg") > mean(&by_visible),
        "prefill rate still divides by visible TTFT"
    );
}

#[test]
fn strategic_points_report_token_totals_and_rates() {
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
            "2",
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
    let completion = point["completion_tokens_total"].as_u64().expect("total");
    assert_eq!(completion, 3 * MAX_TOKENS);
    let prompt = point["prompt_tokens_total"].as_u64().expect("total");
    assert!(prompt > 0);
    // Rates share the stage window behind completion_tokens_per_second.
    let window = completion as f64 / f64_at(point, "completion_tokens_per_second");
    assert_close(
        &point["input_tokens_per_second"],
        prompt as f64 / window,
        "input rate",
    );
    assert_close(
        &point["total_tokens_per_second"],
        (prompt + completion) as f64 / window,
        "total rate",
    );
    // Recompute from the request CSV rows (measured successes only).
    let mut reader = csv::Reader::from_path(&csv).expect("csv");
    let headers = reader.headers().expect("headers").clone();
    let col = |row: &csv::StringRecord, name: &str| -> String {
        let index = headers.iter().position(|h| h == name).expect(name);
        row[index].to_string()
    };
    let rows: Vec<csv::StringRecord> = reader
        .records()
        .map(|row| row.expect("row"))
        .filter(|row| col(row, "success") == "true" && col(row, "warmup") == "false")
        .collect();
    assert_eq!(rows.len(), 3);
    let input: Vec<f64> = rows
        .iter()
        .map(|r| col(r, "input_tokens").parse().expect("input_tokens"))
        .collect();
    let ttft: Vec<f64> = rows
        .iter()
        .map(|r| col(r, "ttft_s").parse().expect("ttft_s"))
        .collect();
    let first_itl: Vec<f64> = rows
        .iter()
        .map(|r| {
            col(r, "itl_s")
                .split(';')
                .next()
                .expect("itl")
                .parse()
                .expect("first itl")
        })
        .collect();
    assert_eq!(prompt as f64, input.iter().sum::<f64>());
    let prefill: Vec<f64> = input.iter().zip(&ttft).map(|(i, t)| i / t).collect();
    let second: Vec<f64> = ttft.iter().zip(&first_itl).map(|(t, i)| t + i).collect();
    assert_eq!(point["prefill_tps_per_user"]["n"], 3);
    assert_close(
        &point["prefill_tps_per_user"]["avg"],
        mean(&prefill),
        "prefill",
    );
    assert_eq!(point["time_to_second_token_s"]["n"], 3);
    assert_close(
        &point["time_to_second_token_s"]["avg"],
        mean(&second),
        "ttst",
    );
    assert!(
        f64_at(&point["time_to_second_token_s"], "avg") > f64_at(&point["ttft_s"], "avg"),
        "second token after first"
    );
}
