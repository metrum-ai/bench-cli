// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Seeded determinism and open-loop arrival checks against dummy-model-server.
//! Skipped if `go` is missing.

mod common;

use common::{request_records, skip, spawn_dummy};
use serde_json::Value;
use std::io::Write;
use std::process::Command;

fn llm_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-llm"))
}

struct Run {
    _dir: tempfile::TempDir,
    data_log: std::path::PathBuf,
}

fn run_llm(url: &str, extra: &[&str]) -> Run {
    let dir = tempfile::tempdir().expect("tmpdir");
    let prompts = dir.path().join("prompts.jsonl");
    {
        let mut file = std::fs::File::create(&prompts).expect("create prompts");
        for i in 0..8 {
            writeln!(file, r#"{{"prompt":"prompt number {i}"}}"#).expect("write prompt");
        }
    }
    let data_log = dir.path().join("out.jsonl");
    let mut args: Vec<String> = [
        "--url",
        url,
        "--api-key",
        "dummy",
        "--scenario",
        "determinism",
        "--num-requests",
        "8",
        "--concurrency",
        "2",
        "--prompts",
        prompts.to_str().unwrap(),
        "--mode",
        "chat",
        "--model",
        "dummy",
        "--max-tokens",
        "8",
        "--data-log",
        data_log.to_str().unwrap(),
        "--debug-log",
        dir.path().join("debug.log").to_str().unwrap(),
        "--error-log",
        dir.path().join("error.log").to_str().unwrap(),
        "--log-level",
        "error",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.extend(extra.iter().map(|s| s.to_string()));

    let status = Command::new(llm_bin())
        .args(&args)
        .status()
        .expect("run llm");
    assert!(status.success(), "llm bench failed");
    Run {
        _dir: dir,
        data_log,
    }
}

/// Scheduled offsets, phases and sequence numbers that depend only on `--seed`
/// must repeat exactly across runs; timings are excluded since they are not
/// reproducible by construction. Offsets are relative to the first scheduled
/// slot of the same phase (in nanoseconds): the warmup barrier (#226) shifts
/// the measured schedule by however long warmup took, and the seed fixes the
/// gaps, not that shift.
fn schedule_shape(data_log: &std::path::Path) -> Vec<(u64, String, Option<i64>)> {
    let records = request_records(data_log);
    // Whole nanoseconds first, then integer subtraction: exact across runs.
    let ns = |r: &serde_json::Value| {
        r["scheduled_offset_s"]
            .as_f64()
            .map(|offset| (offset * 1e9).round() as i64)
    };
    let phase_start = |phase: &str| {
        records
            .iter()
            .filter(|r| r["phase"] == phase)
            .filter_map(ns)
            .min()
            .unwrap_or(0)
    };
    let mut shape: Vec<(u64, String, Option<i64>)> = records
        .iter()
        .map(|record| {
            let phase = record["phase"].as_str().unwrap_or("").to_string();
            let start = phase_start(&phase);
            (
                record["seq"].as_u64().expect("seq"),
                phase,
                ns(record).map(|offset| offset - start),
            )
        })
        .collect();
    shape.sort_by_key(|(seq, _, _)| *seq);
    shape
}

#[test]
fn seeded_poisson_arrivals_are_reproducible() {
    let Some(dummy) = spawn_dummy(&["-chunk-interval", "2ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let url = dummy.url("/v1/chat/completions");
    let flags = [
        "--streaming",
        "--seed",
        "1234",
        "--request-rate",
        "20",
        "--arrival",
        "poisson",
        "--warmup-requests",
        "2",
    ];

    let first = run_llm(&url, &flags);
    let second = run_llm(&url, &flags);
    let third = run_llm(
        &url,
        &[
            "--streaming",
            "--seed",
            "999",
            "--request-rate",
            "20",
            "--arrival",
            "poisson",
            "--warmup-requests",
            "2",
        ],
    );

    let first_shape = schedule_shape(&first.data_log);
    assert_eq!(first_shape.len(), 8, "expected one record per request");
    assert_eq!(
        first_shape,
        schedule_shape(&second.data_log),
        "same seed produced a different schedule"
    );
    assert!(
        first_shape.iter().any(|(_, _, offset)| offset.is_some()),
        "open-loop run recorded no scheduled offsets"
    );
    assert_ne!(
        first_shape,
        schedule_shape(&third.data_log),
        "a different seed produced an identical schedule"
    );

    // Warmup requests are labelled, not silently dropped.
    let warmup = first_shape
        .iter()
        .filter(|(_, phase, _)| phase == "warmup")
        .count();
    assert_eq!(warmup, 2, "expected two warmup-phase records");
}

/// A constant arrival rate spaces the intended offsets evenly, independent of
/// how long each response takes.
#[test]
fn constant_arrival_offsets_match_the_requested_rate() {
    let Some(dummy) = spawn_dummy(&["-latency", "50ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let run = run_llm(
        &dummy.url("/v1/chat/completions"),
        &[
            "--seed",
            "7",
            "--request-rate",
            "10",
            "--arrival",
            "constant",
        ],
    );

    let mut offsets: Vec<(u64, f64)> = request_records(&run.data_log)
        .iter()
        .filter_map(|record: &Value| {
            Some((
                record["seq"].as_u64()?,
                record["scheduled_offset_s"].as_f64()?,
            ))
        })
        .collect();
    offsets.sort_by_key(|(seq, _)| *seq);
    assert_eq!(offsets.len(), 8, "expected a scheduled offset per request");
    for (seq, offset) in &offsets {
        let expected = *seq as f64 / 10.0;
        assert!(
            (offset - expected).abs() < 1e-6,
            "request {seq} scheduled at {offset}s, want {expected}s"
        );
    }
}
