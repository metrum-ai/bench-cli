// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Integration tests: CLI validation and fail-fast behavior (issue #128).
//! Run with: cargo test --test cli_validation

mod common;

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

// Cargo sets CARGO_BIN_EXE_<name> (exact binary name, case preserved) at
// compile time for integration tests, so the binaries are found regardless of
// CARGO_TARGET_DIR. The previous lookup uppercased the name and silently fell
// back to target/debug, which only exists in the default layout.
fn metrum_ai_bench_llm_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-llm"))
}

fn metrum_ai_bench_vlm_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-vlm"))
}

fn metrum_ai_bench_asr_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-asr"))
}

fn metrum_ai_bench_prompts_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-prompts"))
}

fn metrum_ai_bench_strategic_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"))
}

fn metrum_ai_bench_imagegen_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-imagegen"))
}

fn combined_output(out: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    )
}

#[test]
fn metrum_ai_bench_llm_requires_api_key_with_url() {
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(["--url", "http://127.0.0.1:9/v1"])
        .output()
        .expect("run metrum-ai-bench-cli-llm");
    assert!(
        !out.status.success(),
        "metrum-ai-bench-cli-llm --url without --api-key must fail at parse time"
    );
    let text = combined_output(&out);
    assert!(
        text.contains("--api-key"),
        "parse error must name --api-key:\n{text}"
    );
    assert!(
        text.contains("--url"),
        "parse error must name --url:\n{text}"
    );
}

#[test]
fn metrum_ai_bench_vlm_requires_api_key_with_url() {
    let out = Command::new(metrum_ai_bench_vlm_bin())
        .args(["--url", "http://127.0.0.1:9/v1"])
        .output()
        .expect("run metrum-ai-bench-cli-vlm");
    assert!(
        !out.status.success(),
        "metrum-ai-bench-cli-vlm --url without --api-key must fail at parse time"
    );
    let text = combined_output(&out);
    assert!(
        text.contains("--api-key"),
        "parse error must name --api-key:\n{text}"
    );
    assert!(
        text.contains("--url"),
        "parse error must name --url:\n{text}"
    );
}

#[test]
fn metrum_ai_bench_asr_requires_api_key_with_url() {
    let out = Command::new(metrum_ai_bench_asr_bin())
        .args(["--url", "http://127.0.0.1:9/v1"])
        .output()
        .expect("run metrum-ai-bench-cli-asr");
    assert!(
        !out.status.success(),
        "metrum-ai-bench-cli-asr --url without --api-key must fail at parse time"
    );
    let text = combined_output(&out);
    assert!(
        text.contains("--api-key"),
        "parse error must name --api-key:\n{text}"
    );
    assert!(
        text.contains("--url"),
        "parse error must name --url:\n{text}"
    );
}

#[test]
fn metrum_ai_bench_imagegen_requires_api_key_with_url() {
    let out = Command::new(metrum_ai_bench_imagegen_bin())
        .args(["--url", "http://127.0.0.1:9/v1"])
        .output()
        .expect("run metrum-ai-bench-cli-imagegen");
    assert!(
        !out.status.success(),
        "metrum-ai-bench-cli-imagegen --url without --api-key must fail at parse time"
    );
    let text = combined_output(&out);
    assert!(
        text.contains("--api-key"),
        "parse error must name --api-key:\n{text}"
    );
    assert!(
        text.contains("--url"),
        "parse error must name --url:\n{text}"
    );
}

#[test]
fn metrum_ai_bench_llm_endpoints_file_does_not_require_url_or_api_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    let endpoints = dir.path().join("endpoints.yaml");
    std::fs::write(
        &endpoints,
        "endpoints:\n  - url: http://127.0.0.1:9/v1\n    api_key: dummy\n",
    )
    .unwrap();
    let prompts = dir.path().join("prompts.jsonl");
    std::fs::write(&prompts, "{\"prompt\":\"Hi\"}\n").unwrap();
    let data_log = dir.path().join("out.jsonl");

    let out = Command::new(metrum_ai_bench_llm_bin())
        .args([
            "--endpoints-file",
            endpoints.to_str().unwrap(),
            "--scenario",
            "t",
            "--num-requests",
            "1",
            "--concurrency",
            "1",
            "--prompts",
            prompts.to_str().unwrap(),
            "--mode",
            "chat",
            "--model",
            "m",
            "--data-log",
            data_log.to_str().unwrap(),
            "--max-tokens",
            "8",
            "--stop-after-seconds",
            "1",
        ])
        .output()
        .expect("run metrum-ai-bench-cli-llm with endpoints-file");
    let text = combined_output(&out);
    assert!(
        !text.contains("required arguments were not provided"),
        "endpoints-file path must parse without --url/--api-key:\n{text}"
    );
}

#[test]
fn metrum_ai_bench_llm_quiet_prints_one_line_identity() {
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args([
            "--quiet",
            "--url",
            "http://127.0.0.1:9/v1",
            "--api-key",
            "dummy",
            "--scenario",
            "t",
            "--num-requests",
            "1",
            "--concurrency",
            "1",
            "--prompts",
            "missing-prompts.jsonl",
            "--mode",
            "chat",
            "--model",
            "m",
            "--data-log",
            "out.jsonl",
            "--max-tokens",
            "8",
        ])
        .env_remove("NO_BANNER")
        .output()
        .expect("run metrum-ai-bench-cli-llm --quiet");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("@@@"),
        "quiet must suppress ASCII art:\n{stdout}"
    );
    assert!(
        stdout
            .lines()
            .any(|l| l.starts_with("Metrum AI Bench metrum-ai-bench-cli-llm ")),
        "quiet must print one-line identity:\n{stdout}"
    );
}

#[test]
fn metrum_ai_bench_llm_no_banner_env_suppresses_art() {
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(["--url", "http://127.0.0.1:9/v1"])
        .env("NO_BANNER", "1")
        .output()
        .expect("run with NO_BANNER");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("@@@"),
        "NO_BANNER=1 must suppress ASCII art before parse failure:\n{stdout}"
    );
}

#[test]
fn version_only_works_without_skip_env_vars() {
    let mut scrubbed = HashMap::new();
    for (key, value) in std::env::vars() {
        let upper = key.to_ascii_uppercase();
        if upper.starts_with("METRUM_SKIP_") {
            continue;
        }
        scrubbed.insert(key, value);
    }

    let out = Command::new(metrum_ai_bench_asr_bin())
        .args(["--version-only"])
        .env_clear()
        .envs(&scrubbed)
        .output()
        .expect("run metrum-ai-bench-cli-asr --version-only");

    assert!(
        out.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("metrum-ai-bench-cli-asr version"),
        "unexpected version output: {}",
        stdout
    );
}

#[test]
fn metrum_ai_bench_llm_rejects_concurrency_zero() {
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args([
            "--url",
            "http://127.0.0.1:9/v1",
            "--api-key",
            "test",
            "--scenario",
            "t",
            "--num-requests",
            "1",
            "--concurrency",
            "0",
            "--prompts",
            "prompts.jsonl",
            "--mode",
            "chat",
            "--model",
            "m",
            "--data-log",
            "out.jsonl",
            "--max-tokens",
            "16",
        ])
        .output()
        .expect("run metrum-ai-bench-cli-llm");
    assert!(
        !out.status.success(),
        "metrum-ai-bench-cli-llm must reject --concurrency 0"
    );
}

#[test]
fn metrum_ai_bench_llm_rejects_invalid_mode() {
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args([
            "--url",
            "http://127.0.0.1:9/v1",
            "--api-key",
            "test",
            "--scenario",
            "t",
            "--num-requests",
            "1",
            "--concurrency",
            "1",
            "--prompts",
            "prompts.jsonl",
            "--mode",
            "not-a-mode",
            "--model",
            "m",
            "--data-log",
            "out.jsonl",
            "--max-tokens",
            "16",
        ])
        .output()
        .expect("run metrum-ai-bench-cli-llm");
    assert!(
        !out.status.success(),
        "metrum-ai-bench-cli-llm must reject invalid --mode"
    );
}

#[test]
fn metrum_ai_bench_llm_rejects_num_requests_zero() {
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args([
            "--url",
            "http://127.0.0.1:9/v1",
            "--api-key",
            "test",
            "--scenario",
            "t",
            "--num-requests",
            "0",
            "--concurrency",
            "1",
            "--prompts",
            "prompts.jsonl",
            "--mode",
            "chat",
            "--model",
            "m",
            "--data-log",
            "out.jsonl",
            "--max-tokens",
            "16",
        ])
        .output()
        .expect("run metrum-ai-bench-cli-llm");
    assert!(
        !out.status.success(),
        "metrum-ai-bench-cli-llm must reject --num-requests 0"
    );
}

#[test]
fn metrum_ai_bench_vlm_rejects_concurrency_zero() {
    let out = Command::new(metrum_ai_bench_vlm_bin())
        .args([
            "--url",
            "http://127.0.0.1:9/v1",
            "--api-key",
            "test",
            "--scenario",
            "t",
            "--num-requests",
            "1",
            "--concurrency",
            "0",
            "--prompts",
            "prompts.jsonl",
            "--model",
            "m",
            "--data-log",
            "out.jsonl",
            "--max-tokens",
            "16",
        ])
        .output()
        .expect("run metrum-ai-bench-cli-vlm");
    assert!(
        !out.status.success(),
        "metrum-ai-bench-cli-vlm must reject --concurrency 0"
    );
}

#[test]
fn metrum_ai_bench_vlm_rejects_image_cache_size_zero() {
    let out = Command::new(metrum_ai_bench_vlm_bin())
        .args([
            "--url",
            "http://127.0.0.1:9/v1",
            "--api-key",
            "test",
            "--scenario",
            "t",
            "--num-requests",
            "1",
            "--concurrency",
            "1",
            "--prompts",
            "prompts.jsonl",
            "--model",
            "m",
            "--data-log",
            "out.jsonl",
            "--max-tokens",
            "16",
            "--image-cache-size",
            "0",
        ])
        .output()
        .expect("run metrum-ai-bench-cli-vlm");
    assert!(
        !out.status.success(),
        "metrum-ai-bench-cli-vlm must reject --image-cache-size 0"
    );
}

#[test]
fn metrum_ai_bench_vlm_rejects_invalid_image_detail() {
    let out = Command::new(metrum_ai_bench_vlm_bin())
        .args([
            "--url",
            "http://127.0.0.1:9/v1",
            "--api-key",
            "test",
            "--scenario",
            "t",
            "--num-requests",
            "1",
            "--concurrency",
            "1",
            "--prompts",
            "prompts.jsonl",
            "--model",
            "m",
            "--data-log",
            "out.jsonl",
            "--max-tokens",
            "16",
            "--image-detail",
            "not-a-detail",
        ])
        .output()
        .expect("run metrum-ai-bench-cli-vlm");
    assert!(
        !out.status.success(),
        "metrum-ai-bench-cli-vlm must reject invalid --image-detail"
    );
}

#[test]
fn metrum_ai_bench_asr_rejects_concurrency_zero() {
    let out = Command::new(metrum_ai_bench_asr_bin())
        .args([
            "--url",
            "http://127.0.0.1:9/v1",
            "--api-key",
            "test",
            "--scenario",
            "t",
            "--num-requests",
            "1",
            "--concurrency",
            "0",
            "--input",
            "audio.jsonl",
            "--model",
            "whisper-1",
        ])
        .output()
        .expect("run metrum-ai-bench-cli-asr");
    assert!(
        !out.status.success(),
        "metrum-ai-bench-cli-asr must reject --concurrency 0"
    );
}

#[test]
fn metrum_ai_bench_asr_rejects_num_requests_zero() {
    let out = Command::new(metrum_ai_bench_asr_bin())
        .args([
            "--url",
            "http://127.0.0.1:9/v1",
            "--api-key",
            "test",
            "--scenario",
            "t",
            "--num-requests",
            "0",
            "--concurrency",
            "1",
            "--input",
            "audio.jsonl",
            "--model",
            "whisper-1",
        ])
        .output()
        .expect("run metrum-ai-bench-cli-asr");
    assert!(
        !out.status.success(),
        "metrum-ai-bench-cli-asr must reject --num-requests 0"
    );
}

#[test]
fn metrum_ai_bench_asr_rejects_invalid_response_format() {
    let out = Command::new(metrum_ai_bench_asr_bin())
        .args([
            "--url",
            "http://127.0.0.1:9/v1",
            "--api-key",
            "test",
            "--scenario",
            "t",
            "--num-requests",
            "1",
            "--concurrency",
            "1",
            "--input",
            "audio.jsonl",
            "--model",
            "whisper-1",
            "--response-format",
            "not-a-format",
        ])
        .output()
        .expect("run metrum-ai-bench-cli-asr");
    assert!(
        !out.status.success(),
        "metrum-ai-bench-cli-asr must reject invalid --response-format"
    );
}

#[test]
fn metrum_ai_bench_prompts_version_only() {
    let out = Command::new(metrum_ai_bench_prompts_bin())
        .args(["--version-only"])
        .output()
        .expect("run metrum-ai-bench-cli-prompts --version-only");
    assert!(
        out.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("metrum-ai-bench-cli-prompts version"));
}

#[test]
fn metrum_ai_bench_prompts_version_flag() {
    let out = Command::new(metrum_ai_bench_prompts_bin())
        .args(["--version"])
        .output()
        .expect("run metrum-ai-bench-cli-prompts --version");
    assert!(
        out.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let combined = combined_output(&out);
    assert!(
        combined.contains(env!("CARGO_PKG_VERSION")),
        "unexpected version output: {combined}"
    );
}

#[test]
fn metrum_ai_bench_strategic_version_flags() {
    for flag in ["--version", "--version-only", "-V"] {
        let out = Command::new(metrum_ai_bench_strategic_bin())
            .args([flag])
            .output()
            .unwrap_or_else(|_| panic!("run metrum-ai-bench-cli-strategic {flag}"));
        assert!(
            out.status.success(),
            "{flag} stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let combined = combined_output(&out);
        assert!(
            combined.contains(env!("CARGO_PKG_VERSION")),
            "{flag} unexpected output: {combined}"
        );
    }
    let only = Command::new(metrum_ai_bench_strategic_bin())
        .args(["--version-only"])
        .output()
        .expect("strategic --version-only");
    let stdout = String::from_utf8_lossy(&only.stdout);
    assert!(stdout.contains("metrum-ai-bench-cli-strategic version"));
}

#[test]
fn metrum_ai_bench_prompts_selects_from_local_jsonl() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("lib.jsonl");
    let output = dir.path().join("mix.jsonl");
    let report = dir.path().join("report.json");
    let mut body = String::new();
    for i in 0..8 {
        body.push_str(&format!(
            "{{\"prompt\":\"row {i}\",\"target_output_length\":20,\"reasoning\":false,\"target_input_tokens\":64,\"target_output_tokens\":128,\"source_ordinal\":{i}}}\n"
        ));
    }
    std::fs::write(&input, body).expect("write fixture");

    let out = Command::new(metrum_ai_bench_prompts_bin())
        .args([
            "--local-jsonl",
            input.to_str().unwrap(),
            "--count",
            "4",
            "--count-slack",
            "0",
            "--seed",
            "7",
            "--isl-target",
            "64",
            "--isl-unit",
            "tokens",
            "--isl-stat",
            "mean",
            "--isl-tolerance",
            "0",
            "--osl-target",
            "128",
            "--osl-unit",
            "tokens",
            "--osl-stat",
            "mean",
            "--osl-tolerance",
            "0",
            "--max-repeats",
            "1",
            "--output",
            output.to_str().unwrap(),
            "--report",
            report.to_str().unwrap(),
        ])
        .output()
        .expect("run metrum-ai-bench-cli-prompts");
    assert!(
        out.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let report_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
    assert_eq!(report_json["selected_count"], 4);
    assert_eq!(report_json["recommended_num_requests"], 4);
    assert_eq!(report_json["recommended_max_tokens"], 128);
    let mix = std::fs::read_to_string(&output).unwrap();
    assert_eq!(mix.lines().count(), 4);
    assert!(mix.contains("Please aim for approximately 20 words"));
}

#[test]
fn metrum_ai_bench_prompts_fails_before_writing_when_impossible() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("lib.jsonl");
    let output = dir.path().join("mix.jsonl");
    let report = dir.path().join("report.json");
    std::fs::write(
        &input,
        r#"{"prompt":"only","target_output_length":20,"reasoning":false,"target_input_tokens":32,"target_output_tokens":32,"source_ordinal":0}
"#,
    )
    .unwrap();

    let out = Command::new(metrum_ai_bench_prompts_bin())
        .args([
            "--local-jsonl",
            input.to_str().unwrap(),
            "--count",
            "4",
            "--count-slack",
            "0",
            "--no-repeats",
            "--seed",
            "1",
            "--isl-target",
            "1024",
            "--isl-tolerance",
            "0",
            "--osl-target",
            "1024",
            "--osl-tolerance",
            "0",
            "--output",
            output.to_str().unwrap(),
            "--report",
            report.to_str().unwrap(),
        ])
        .output()
        .expect("run metrum-ai-bench-cli-prompts");
    assert!(!out.status.success());
    assert!(!output.exists(), "must not write JSONL on failure");
    assert!(!report.exists(), "must not write report on failure");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no mix within ISL/OSL") || stderr.contains("kind:"),
        "stderr:\n{stderr}"
    );
}

#[test]
fn llm_rejects_warmup_that_consumes_all_requests() {
    let tmp = tempfile::tempdir().unwrap();
    let prompts = tmp.path().join("prompts.jsonl");
    std::fs::write(&prompts, r#"{"prompt":"Hi"}"#).unwrap();
    let data_log = tmp.path().join("out.jsonl");
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args([
            "--url",
            "http://127.0.0.1:9/v1/chat/completions",
            "--api-key",
            "dummy",
            "--scenario",
            "empty-warmup",
            "--num-requests",
            "4",
            "--warmup-requests",
            "4",
            "--concurrency",
            "1",
            "--prompts",
            prompts.to_str().unwrap(),
            "--mode",
            "chat",
            "--streaming",
            "--model",
            "dummy",
            "--max-tokens",
            "8",
            "--data-log",
            data_log.to_str().unwrap(),
            "--debug-log",
            tmp.path().join("d.log").to_str().unwrap(),
            "--error-log",
            tmp.path().join("e.log").to_str().unwrap(),
            "--log-level",
            "error",
        ])
        .output()
        .expect("run llm");
    assert!(
        !out.status.success(),
        "warmup >= num_requests must fail before requests"
    );
    assert!(
        !data_log.exists(),
        "must not write a data log when the empty-run gate fires"
    );
    let text = combined_output(&out);
    assert!(
        text.contains("warmup_requests (4)") && text.contains("num_requests (4)"),
        "error must name both counts:\n{text}"
    );
}

#[test]
fn strategic_rejects_zero_requests_per_stage() {
    let out = Command::new(metrum_ai_bench_strategic_bin())
        .args([
            "--url",
            "http://127.0.0.1:9/v1/chat/completions",
            "--api-key",
            "dummy",
            "--model",
            "dummy",
            "--requests-per-stage",
            "0",
            "--sweep",
            "1",
        ])
        .output()
        .expect("run strategic");
    assert!(!out.status.success());
    let text = combined_output(&out);
    assert!(
        text.contains("requests_per_stage") || text.contains("range"),
        "must reject zero measured requests:\n{text}"
    );
}

// #313: effective prompt-sequence emission/replay, cohort diagnostics,
// bounded SSE trace, and reproducible start jitter.

fn seq_run_args(url: &str, data_log: &str, extra: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = [
        "--url",
        url,
        "--api-key",
        "dummy",
        "--scenario",
        "sequence",
        "--num-requests",
        "6",
        "--concurrency",
        "2",
        "--prompts",
        "test-data/llm-hi.jsonl",
        "--mode",
        "chat",
        "--model",
        "m",
        "--data-log",
        data_log,
        "--max-tokens",
        "8",
        "--log-level",
        "error",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.extend(extra.iter().map(|s| s.to_string()));
    args
}

#[test]
fn prompt_sequence_log_emits_effective_order_without_content() {
    let mock = common::spawn_mock(&[]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let seq_log = dir.path().join("seq.jsonl");
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(seq_run_args(
            &url,
            data_log.to_str().unwrap(),
            &["--prompt-sequence-log", seq_log.to_str().unwrap()],
        ))
        .output()
        .expect("run with sequence log");
    assert!(out.status.success());
    let text = std::fs::read_to_string(&seq_log).unwrap();
    let rows: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(rows.len(), 6, "one row per slot");
    assert_eq!(rows[0]["seq"], 0);
    assert_eq!(rows[0]["cohort"], 0, "cohort = seq / cap");
    assert_eq!(rows[4]["cohort"], 2);
    assert!(rows[0]["prompt_sha256_16"].as_str().unwrap().len() == 16);
    assert_eq!(
        rows[0]["prompt_bytes"].as_u64(),
        Some(2),
        "llm-hi.jsonl prompt length"
    );
    assert!(
        !text.contains("Hi"),
        "no prompt text in the sequence log: {text}"
    );
}

#[test]
fn prompt_sequence_replay_reproduces_the_recorded_order() {
    let mock = common::spawn_mock(&[]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let log_a = dir.path().join("seq-a.jsonl");
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(seq_run_args(
            &url,
            dir.path().join("a.jsonl").to_str().unwrap(),
            &["--prompt-sequence-log", log_a.to_str().unwrap()],
        ))
        .output()
        .expect("run A");
    assert!(out.status.success());

    // Replay A's effective order with a DIFFERENT seed; the replayed
    // sequence log must match A exactly (seq → fingerprint).
    let log_b = dir.path().join("seq-b.jsonl");
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(seq_run_args(
            &url,
            dir.path().join("b.jsonl").to_str().unwrap(),
            &[
                "--prompt-sequence",
                log_a.to_str().unwrap(),
                "--prompt-sequence-log",
                log_b.to_str().unwrap(),
                "--seed",
                "999",
            ],
        ))
        .output()
        .expect("run B replay");
    assert!(
        out.status.success(),
        "replay must succeed:\n{}",
        combined_output(&out)
    );
    assert_eq!(
        std::fs::read_to_string(&log_a).unwrap(),
        std::fs::read_to_string(&log_b).unwrap(),
        "replayed sequence reproduces the recorded order exactly"
    );
}

#[test]
fn prompt_sequence_replay_rejects_unmatched_content() {
    let mock = common::spawn_mock(&[]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let seq = dir.path().join("foreign.jsonl");
    std::fs::write(
        &seq,
        "{\"seq\":0,\"prompt_index\":0,\"prompt_sha256_16\":\"deadbeef00000000\",\"prompt_bytes\":99,\"cohort\":0}\n",
    )
    .unwrap();
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(seq_run_args(
            &url,
            dir.path().join("out.jsonl").to_str().unwrap(),
            &["--prompt-sequence", seq.to_str().unwrap()],
        ))
        .output()
        .expect("run replay with foreign content");
    assert!(
        !out.status.success(),
        "replaying content that is not in the pool must fail before traffic"
    );
    let text = combined_output(&out);
    assert!(
        text.contains("prompt"),
        "error names the sequence problem:\n{text}"
    );
}

#[test]
fn start_jitter_staggers_sends_and_is_stamped() {
    let mock = common::spawn_mock(&["--latency-ms", "20"]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(seq_run_args(
            &url,
            data_log.to_str().unwrap(),
            &["--start-jitter-ms", "400", "--start-jitter-seed", "1234"],
        ))
        .output()
        .expect("run with jitter");
    assert!(out.status.success());
    let text = std::fs::read_to_string(&data_log).unwrap();
    let rows: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let offsets: Vec<f64> = rows
        .iter()
        .filter(|r| {
            r["schema_version"]
                .as_str()
                .unwrap_or("")
                .contains(".request.")
        })
        .filter_map(|r| r["send_offset_s"].as_f64())
        .collect();
    let spread = offsets.iter().fold(f64::MIN, |a, &b| a.max(b))
        - offsets.iter().fold(f64::MAX, |a, &b| a.min(b));
    assert!(
        spread > 0.05,
        "jitter staggers the closed-loop sends: spread {spread}s over {offsets:?}"
    );
    let summary = rows.last().unwrap();
    assert_eq!(
        summary["config"]["modality"]["start_jitter_ms"], 400,
        "jitter is recorded in the run config"
    );
    assert_eq!(summary["config"]["modality"]["start_jitter_seed"], 1234);
}

#[test]
fn sse_trace_first_n_writes_bounded_redacted_timelines() {
    let mock = common::spawn_mock(&["--latency-ms", "30"]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(seq_run_args(
            &url,
            data_log.to_str().unwrap(),
            &["--streaming", "--sse-trace-first-n", "2"],
        ))
        .output()
        .expect("run with SSE trace");
    assert!(
        out.status.success(),
        "traced streaming run must succeed:\n{}",
        combined_output(&out)
    );
    let trace_path = format!("{}.sse-trace.jsonl", data_log.to_str().unwrap());
    let text = std::fs::read_to_string(&trace_path).expect("trace file written");
    let rows: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(rows.len(), 2, "bounded to the first N requests");
    for row in &rows {
        assert_eq!(row["schema_version"], "metrum-ai-bench-cli.sse-trace.v1");
        let events = row["events"].as_array().unwrap();
        assert!(!events.is_empty(), "traced requests have events");
        for event in events {
            assert!(event["t_ms"].as_u64().is_some(), "timestamps only");
            assert!(event["categories"].as_array().is_some(), "categories only");
        }
    }
    let all = std::fs::read_to_string(&trace_path).unwrap();
    assert!(
        !all.contains("Hi") && !all.contains("hello"),
        "no content in the trace: {all}"
    );
}
