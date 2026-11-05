// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Local-mock mode and endpoint certification.
mod common;

use common::{request_records, spawn_dummy, summary_record};
use std::process::Command;

// BC-MODE-002: unary output clocks, explicit header approximation and errors.
#[test]
fn bc_mode_002_nonstreaming_clocks_and_outcomes() {
    let healthy = spawn_dummy(&["-latency", "1ms"]).expect("catalog requires dummy-model-server");
    let failing = spawn_dummy(&["-error-rate", "1"]).expect("catalog requires failing dummy");
    for (dummy, failure, infer) in [
        (&healthy, false, false),
        (&healthy, false, true),
        (&failing, true, false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let prompts = dir.path().join("prompts.jsonl");
        let data = dir.path().join("rows.jsonl");
        std::fs::write(&prompts, "{\"prompt\":\"hello\"}\n").unwrap();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-llm"));
        cmd.args([
            "--url",
            &dummy.url("/v1/chat/completions"),
            "--api-key",
            "dummy",
            "--scenario",
            "catalog",
            "--num-requests",
            "1",
            "--concurrency",
            "1",
            "--mode",
            "chat",
            "--model",
            "stub",
            "--max-tokens",
            "8",
            "--request-timeout",
            "3",
            "--fail-on-error",
            "--log-level",
            "error",
        ])
        .arg("--prompts")
        .arg(&prompts)
        .arg("--data-log")
        .arg(&data)
        .arg("--debug-log")
        .arg(dir.path().join("debug.log"))
        .arg("--error-log")
        .arg(dir.path().join("error.log"));
        if infer {
            cmd.arg("--infer-ttft-from-first-byte");
        }
        let output = cmd.output().unwrap();
        assert_eq!(
            output.status.success(),
            !failure,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let records = request_records(&data);
        assert_eq!(records.len(), 1);
        let row = &records[0];
        assert!(row["ttft_visible_answer_s"].is_null() && row["ttft_first_any_s"].is_null());
        if failure {
            assert_eq!(row["error"]["kind"], "http_status");
            assert!(row["answer_completed"].is_null());
        } else {
            assert_eq!(row["outcome"], "answer");
            assert_eq!(row["answer_completed"], true);
            assert_eq!(row["usage_missing"], false);
            assert_eq!(row["usage_prompt_tokens_present"], true);
            assert_eq!(row["usage_completion_tokens_present"], true);
            if infer {
                assert_eq!(row["ttft_source"], "first_byte_approx");
                assert_eq!(row["ttft_s"], row["first_byte_s"]);
            } else {
                assert!(row["ttft_s"].is_null());
            }
        }
        let csv = dir.path().join("strategic.csv");
        let mut strategic = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"));
        strategic
            .args([
                "--url",
                &dummy.url("/v1/chat/completions"),
                "--api-key",
                "dummy",
                "--model",
                "stub",
                "--max-tokens",
                "8",
                "--sweep",
                "1",
                "--requests-per-stage",
                "1",
                "--fail-on-error",
            ])
            .arg("--csv")
            .arg(&csv)
            .arg("--html")
            .arg(dir.path().join("report.html"));
        if infer {
            strategic.arg("--infer-ttft-from-first-byte");
        }
        let output = strategic.output().unwrap();
        assert_eq!(
            output.status.success(),
            !failure,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let rows: Vec<metrum_ai_bench::strategic::BenchRecord> = csv::Reader::from_path(csv)
            .unwrap()
            .deserialize()
            .map(Result::unwrap)
            .collect();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert!(row.ttft_visible_answer_s.is_none() && row.first_any_s().is_none());
        if failure {
            assert!(!row.success && row.error.is_some());
        } else {
            assert_eq!(row.answer_completed, Some(true));
            assert!(!row.usage_missing);
            assert_eq!(row.usage_prompt_tokens_present, Some(true));
            assert_eq!(row.usage_completion_tokens_present, Some(true));
            if infer {
                assert_eq!(
                    row.ttft_source,
                    Some(metrum_ai_bench::measurement::TtftSource::FirstByteApprox)
                );
                assert_eq!(row.ttft_s, row.first_byte_s);
            } else {
                assert!(row.ttft_s.is_none());
            }
        }
    }
}

// BC-MODE-003: fixed round-robin requests, one failed and both healthy.
#[test]
fn bc_mode_003_multiple_endpoints_and_safety() {
    let first = spawn_dummy(&["-latency", "1ms"]).expect("catalog requires dummy-model-server");
    let healthy = spawn_dummy(&["-latency", "1ms"]).expect("catalog requires second dummy");
    let failing = spawn_dummy(&["-error-rate", "1"]).expect("catalog requires failing dummy");
    for (second, fails, flags) in [
        (&healthy, false, &[][..]),
        (&failing, true, &[][..]),
        (&failing, true, &["--fail-on-error"][..]),
        (&failing, true, &["--ci-safety"][..]),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let endpoints = dir.path().join("endpoints.yaml");
        std::fs::write(&endpoints, format!("- name: first\n  url: {}\n  api_key: dummy\n- name: second\n  url: {}\n  api_key: dummy\n", first.url("/v1/chat/completions"), second.url("/v1/chat/completions"))).unwrap();
        let prompts = dir.path().join("prompts.jsonl");
        let data = dir.path().join("out.jsonl");
        std::fs::write(&prompts, "{\"prompt\":\"hello\"}\n").unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-llm"))
            .args([
                "--endpoints-file",
                endpoints.to_str().unwrap(),
                "--scenario",
                "catalog",
                "--num-requests",
                "4",
                "--concurrency",
                "1",
                "--mode",
                "chat",
                "--model",
                "stub",
                "--max-tokens",
                "8",
                "--request-timeout",
                "3",
                "--log-level",
                "error",
            ])
            .arg("--prompts")
            .arg(&prompts)
            .arg("--data-log")
            .arg(&data)
            .arg("--debug-log")
            .arg(dir.path().join("debug.log"))
            .arg("--error-log")
            .arg(dir.path().join("error.log"))
            .args(flags)
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            !fails || flags.is_empty(),
            "{flags:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let records = request_records(&data);
        assert_eq!(records.len(), 4);
        let first_rows: Vec<_> = records
            .iter()
            .filter(|r| r["endpoint"] == "first")
            .collect();
        let second_rows: Vec<_> = records
            .iter()
            .filter(|r| r["endpoint"] == "second")
            .collect();
        assert_eq!((first_rows.len(), second_rows.len()), (2, 2));
        assert!(first_rows.iter().all(|r| r["error"].is_null()));
        assert!(second_rows.iter().all(|r| r["error"].is_null() != fails));
        let summary = summary_record(&data).unwrap();
        assert_eq!(summary["attempted"], 4);
        assert_eq!(summary["successes"], if fails { 2 } else { 4 });
        assert_eq!(summary["errors"], if fails { 2 } else { 0 });
        assert_eq!(summary["per_endpoint"]["first"]["attempted"], 2);
        assert_eq!(
            summary["per_endpoint"]["second"]["errors"],
            if fails { 2 } else { 0 }
        );
    }
}
