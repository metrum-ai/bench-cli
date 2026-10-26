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

// #304: invalid absolute URLs must be rejected before any network traffic.
// Scheme-less --url values previously produced an all-error benchmark run
// that still exited 0.

/// Minimal full argument set for metrum-ai-bench-cli-llm so clap parse
/// succeeds and the run reaches endpoint resolution.
fn llm_args(url: &str, data_log: &str) -> Vec<String> {
    [
        "--url",
        url,
        "--api-key",
        "dummy",
        "--scenario",
        "url-validation",
        "--num-requests",
        "1",
        "--concurrency",
        "1",
        "--prompts",
        "missing-is-fine-before-url-check.jsonl",
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
    .collect()
}

#[test]
fn metrum_ai_bench_llm_rejects_scheme_less_url_before_traffic() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let args = llm_args(
        "127.0.0.1:9/v1/chat/completions",
        data_log.to_str().unwrap(),
    );
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(&args)
        .output()
        .expect("run metrum-ai-bench-cli-llm with scheme-less --url");
    assert_eq!(
        out.status.code(),
        Some(2),
        "scheme-less --url must exit 2 (configuration error), not benchmark a broken endpoint"
    );
    let text = combined_output(&out);
    assert!(
        text.contains("invalid endpoint URL"),
        "error must explain the invalid URL:\n{text}"
    );
    assert!(
        text.contains("127.0.0.1:9"),
        "error must name the offending URL:\n{text}"
    );
    // Fail-fast proof: the run never reaches request issuance or logging.
    assert!(
        !text.contains("Starting load test"),
        "must fail before the load loop starts:\n{text}"
    );
    assert!(
        !data_log.exists(),
        "must fail before the data log is created: {}",
        data_log.display()
    );
}

#[test]
fn metrum_ai_bench_llm_accepts_proxy_path_urls() {
    // A URL with a proxy path prefix is valid; the (closed) port only fails
    // later at request time, which this test does not reach because the
    // prompts file is missing after validation.
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let args = llm_args(
        "http://127.0.0.1:9/llm-proxy/v1/chat/completions",
        data_log.to_str().unwrap(),
    );
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(&args)
        .output()
        .expect("run metrum-ai-bench-cli-llm with proxy-path --url");
    let text = combined_output(&out);
    assert!(
        !text.contains("invalid endpoint URL"),
        "proxy path URLs must pass URL validation:\n{text}"
    );
}

#[test]
fn metrum_ai_bench_vlm_rejects_scheme_less_url_before_traffic() {
    let out = Command::new(metrum_ai_bench_vlm_bin())
        .args([
            "--url",
            "127.0.0.1:9/v1/chat/completions",
            "--api-key",
            "dummy",
            "--scenario",
            "t",
            "--num-requests",
            "1",
            "--concurrency",
            "1",
            "--prompts",
            "missing.jsonl",
            "--model",
            "m",
            "--data-log",
            "out-vlm.jsonl",
            "--max-tokens",
            "8",
        ])
        .output()
        .expect("run metrum-ai-bench-cli-vlm with scheme-less --url");
    assert_eq!(out.status.code(), Some(2), "must exit 2 before traffic");
    let text = combined_output(&out);
    assert!(
        text.contains("invalid endpoint URL"),
        "error must explain the invalid URL:\n{text}"
    );
}

#[test]
fn metrum_ai_bench_asr_rejects_scheme_less_url_before_traffic() {
    let out = Command::new(metrum_ai_bench_asr_bin())
        .args([
            "--url",
            "127.0.0.1:9/v1",
            "--api-key",
            "dummy",
            "--scenario",
            "t",
            "--num-requests",
            "1",
            "--input",
            "missing-is-fine-before-url-check.jsonl",
            "--model",
            "whisper-1",
        ])
        .output()
        .expect("run metrum-ai-bench-cli-asr with scheme-less --url");
    assert_eq!(out.status.code(), Some(2), "must exit 2 before traffic");
    let text = combined_output(&out);
    assert!(
        text.contains("invalid endpoint URL"),
        "error must explain the invalid URL:\n{text}"
    );
}

#[test]
fn metrum_ai_bench_imagegen_rejects_scheme_less_url_before_traffic() {
    let out = Command::new(metrum_ai_bench_imagegen_bin())
        .args([
            "--url",
            "127.0.0.1:9/v1",
            "--api-key",
            "dummy",
            "--scenario",
            "t",
            "--model",
            "m",
            "--num-requests",
            "1",
            "--concurrency",
            "1",
            "--prompt",
            "a square",
            "--data-log",
            "out-imagegen.jsonl",
        ])
        .output()
        .expect("run metrum-ai-bench-cli-imagegen with scheme-less --url");
    assert_eq!(out.status.code(), Some(2), "must exit 2 before traffic");
    let text = combined_output(&out);
    assert!(
        text.contains("invalid endpoint URL"),
        "error must explain the invalid URL:\n{text}"
    );
}

// #304: opt-in exact-model preflight (--validate-model), tested against the
// workspace mock server (serves /v1/models with "metrum-ai-bench-cli-mock").

#[test]
fn metrum_ai_bench_llm_invalid_url_redacts_embedded_credentials() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let args = llm_args(
        "ftp://user:secret@h.example/llm/v1/chat/completions?api_key=hush",
        data_log.to_str().unwrap(),
    );
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(&args)
        .output()
        .expect("run metrum-ai-bench-cli-llm with credentials in --url");
    assert_eq!(
        out.status.code(),
        Some(2),
        "non-http scheme must exit 2 before traffic"
    );
    let text = combined_output(&out);
    assert!(
        !text.contains("secret") && !text.contains("hush"),
        "no password or query credential may be echoed:\n{text}"
    );
    assert!(
        text.contains("api_key=REDACTED"),
        "query keys stay, values are redacted:\n{text}"
    );
}

#[test]
fn metrum_ai_bench_llm_validate_model_redacts_urls_in_failure_output() {
    // Unreachable server with credentials in the URL: the preflight failure
    // (stderr + report) must not leak them.
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let mut args = llm_args(
        "http://user:hush@127.0.0.1:1/v1/chat/completions?token=shh",
        data_log.to_str().unwrap(),
    );
    args.push("--validate-model".into());
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(&args)
        .output()
        .expect("run metrum-ai-bench-cli-llm --validate-model with credentials in --url");
    assert_eq!(out.status.code(), Some(2), "must fail preflight");
    let text = combined_output(&out);
    assert!(
        !text.contains("hush") && !text.contains("shh"),
        "userinfo and query credentials must be redacted from the failure output:\n{text}"
    );
    assert!(
        text.contains("/v1/models"),
        "the redacted models URL is still shown:\n{text}"
    );
}

#[test]
fn metrum_ai_bench_llm_validate_model_reports_unreachable_server() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let mut args = llm_args(
        "http://127.0.0.1:1/v1/chat/completions",
        data_log.to_str().unwrap(),
    );
    args.push("--validate-model".into());
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(&args)
        .output()
        .expect("run metrum-ai-bench-cli-llm --validate-model against closed port");
    assert!(
        !out.status.success(),
        "unreachable server must fail preflight"
    );
    let text = combined_output(&out);
    assert!(
        text.contains("model preflight failed"),
        "must name the preflight failure:\n{text}"
    );
    assert!(
        text.contains("/v1/models"),
        "must name the models URL that could not be reached:\n{text}"
    );
}

#[test]
fn metrum_ai_bench_llm_validate_model_reports_actual_server_models() {
    let mock = common::spawn_mock(&[]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let mut args = llm_args(&url, data_log.to_str().unwrap());
    args.push("--validate-model".into());
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(&args)
        .output()
        .expect("run metrum-ai-bench-cli-llm --validate-model with unknown model");
    assert_eq!(
        out.status.code(),
        Some(2),
        "unknown model must fail preflight with the configuration code"
    );
    let text = combined_output(&out);
    assert!(
        text.contains("model preflight failed"),
        "must name the preflight failure:\n{text}"
    );
    // The report must include the server's actual model list.
    assert!(
        text.contains("metrum-ai-bench-cli-mock"),
        "must report the actual server model ids:\n{text}"
    );
}

#[test]
fn metrum_ai_bench_llm_validate_model_passes_and_runs() {
    let mock = common::spawn_mock(&[]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let prompts = dir.path().join("prompts.jsonl");
    std::fs::write(&prompts, "{\"prompt\":\"Hi\"}\n").unwrap();
    let data_log = dir.path().join("out.jsonl");
    let mut args = llm_args(&url, data_log.to_str().unwrap());
    // Point prompts at a real file so the run completes.
    if let Some(pos) = args
        .iter()
        .position(|a| a == "missing-is-fine-before-url-check.jsonl")
    {
        args[pos] = prompts.to_str().unwrap().to_string();
    }
    args.push("--validate-model".into());
    // The mock serves exactly "metrum-ai-bench-cli-mock".
    if let Some(pos) = args.iter().position(|a| a == "m") {
        args[pos] = "metrum-ai-bench-cli-mock".into();
    }
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(&args)
        .output()
        .expect("run metrum-ai-bench-cli-llm --validate-model happy path");
    assert!(
        out.status.success(),
        "listed model must pass preflight and complete the run:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("model preflight: ok"),
        "preflight success must be reported on stdout:\n{stdout}"
    );
    let logged = std::fs::read_to_string(&data_log).expect("data log written");
    let lines: Vec<&str> = logged.lines().collect();
    assert_eq!(lines.len(), 2, "one request row and one summary: {logged}");
    assert!(
        lines[0].contains("\"seq\""),
        "a measured request record must appear first: {logged}"
    );
    assert!(
        lines[1].contains("metrum-ai-bench-cli.summary.v3"),
        "the closing summary row must be present: {logged}"
    );
}

#[test]
fn metrum_ai_bench_llm_validate_model_alias_overrides_checked_id() {
    let mock = common::spawn_mock(&[]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let mut args = llm_args(&url, data_log.to_str().unwrap());
    args.push("--validate-model".into());
    // Run with model "m", validate the server-known alias instead (alias/
    // proxy override for proxies that rewrite model names).
    args.push("--model-alias".into());
    args.push("metrum-ai-bench-cli-mock".into());
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(&args)
        .output()
        .expect("run metrum-ai-bench-cli-llm --validate-model --model-alias");
    // The prompts file is missing, so the run fails AFTER preflight passes.
    let text = combined_output(&out);
    assert!(
        text.contains("model preflight: ok"),
        "alias must be validated and pass against the mock server:\n{text}"
    );
    assert!(
        text.contains("'metrum-ai-bench-cli-mock' listed"),
        "preflight success line must name the validated alias:\n{text}"
    );
    assert!(
        !out.status.success(),
        "run must still fail on missing prompts"
    );
}

#[test]
fn metrum_ai_bench_llm_model_alias_requires_validate_model() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let mut args = llm_args(
        "http://127.0.0.1:9/v1/chat/completions",
        data_log.to_str().unwrap(),
    );
    args.push("--model-alias".into());
    args.push("some-alias".into());
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(&args)
        .output()
        .expect("run metrum-ai-bench-cli-llm --model-alias without --validate-model");
    assert!(
        !out.status.success(),
        "--model-alias must require --validate-model"
    );
    let text = combined_output(&out);
    assert!(
        text.contains("--validate-model"),
        "parse error must name --validate-model:\n{text}"
    );
}
// #305: explicit output collision policy for --data-log.

/// Full metrum-ai-bench-cli-llm argument set for a one-request run against
/// `url`, writing `data_log`, with `extra` args appended.
fn collision_args(url: &str, data_log: &str, extra: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = [
        "--url",
        url,
        "--api-key",
        "dummy",
        "--scenario",
        "collision-policy",
        "--num-requests",
        "1",
        "--concurrency",
        "1",
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

/// Run metrum-ai-bench-cli-llm once against `url` with one request and
/// `extra` args appended; return the exit status.
fn run_llm_once(url: &str, data_log: &str, extra: &[&str]) -> std::process::ExitStatus {
    Command::new(metrum_ai_bench_llm_bin())
        .args(collision_args(url, data_log, extra))
        .status()
        .expect("run metrum-ai-bench-cli-llm")
}

/// Distinct run_ids present in a data log (one per summary row).
fn data_log_run_ids(path: &std::path::Path) -> std::collections::BTreeSet<String> {
    let text = std::fs::read_to_string(path).expect("data log readable");
    text.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|row| {
            row.get("run_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .collect()
}

#[test]
fn data_log_default_is_append_across_runs() {
    let mock = common::spawn_mock(&[]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let status = run_llm_once(&url, data_log.to_str().unwrap(), &[]);
    assert!(status.success());
    let status = run_llm_once(&url, data_log.to_str().unwrap(), &[]);
    assert!(status.success());
    let text = std::fs::read_to_string(&data_log).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines.len(),
        4,
        "two runs of 1 request + summary each: {text}"
    );
    assert_eq!(
        data_log_run_ids(&data_log).len(),
        2,
        "appended runs stay attributable by unique run_id: {text}"
    );
}

#[test]
fn data_log_refuse_existing_exits_2_before_traffic() {
    let mock = common::spawn_mock(&[]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let status = run_llm_once(&url, data_log.to_str().unwrap(), &[]);
    assert!(status.success());
    let before = std::fs::read_to_string(&data_log).unwrap();
    let out = Command::new(metrum_ai_bench_llm_bin())
        .env_remove("NO_BANNER")
        .args(collision_args(
            &url,
            data_log.to_str().unwrap(),
            &["--data-log-mode", "refuse-existing"],
        ))
        .output()
        .expect("run with refuse-existing");
    assert_eq!(
        out.status.code(),
        Some(2),
        "existing data log must be refused with the configuration code"
    );
    let text = combined_output(&out);
    assert!(
        text.contains("already exists"),
        "refusal must name the collision:\n{text}"
    );
    assert_eq!(
        std::fs::read_to_string(&data_log).unwrap(),
        before,
        "refuse-existing must not touch the existing log"
    );
}

#[test]
fn data_log_overwrite_never_mixes_runs() {
    let mock = common::spawn_mock(&[]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let status = run_llm_once(&url, data_log.to_str().unwrap(), &[]);
    assert!(status.success());
    let status = run_llm_once(
        &url,
        data_log.to_str().unwrap(),
        &["--data-log-mode", "overwrite"],
    );
    assert!(status.success());
    let text = std::fs::read_to_string(&data_log).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "overwrite keeps exactly the new run: {text}"
    );
    assert_eq!(
        data_log_run_ids(&data_log).len(),
        1,
        "no previous-run rows may survive overwrite: {text}"
    );
}

#[test]
fn data_log_new_dir_writes_one_file_per_run() {
    let mock = common::spawn_mock(&[]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let runs_dir = dir.path().join("runs");
    for _ in 0..2 {
        let status = run_llm_once(
            &url,
            runs_dir.to_str().unwrap(),
            &["--data-log-mode", "new-dir"],
        );
        assert!(status.success());
    }
    let entries: Vec<_> = std::fs::read_dir(&runs_dir)
        .expect("new-dir creates the directory")
        .collect();
    assert_eq!(entries.len(), 2, "one file per run: {entries:?}");
    for entry in &entries {
        let text = std::fs::read_to_string(entry.as_ref().unwrap().path()).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "each per-run file holds one run: {text}");
    }
    let ids: std::collections::BTreeSet<String> = entries
        .iter()
        .map(|entry| {
            data_log_run_ids(&entry.as_ref().unwrap().path())
                .into_iter()
                .next()
                .unwrap()
        })
        .collect();
    assert_eq!(ids.len(), 2, "per-run files are named by distinct run_ids");
}

#[test]
fn data_log_mode_rejects_unknown_value() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(collision_args(
            "http://127.0.0.1:9/v1/chat/completions",
            data_log.to_str().unwrap(),
            &["--data-log-mode", "bogus"],
        ))
        .output()
        .expect("run with bogus mode");
    assert_eq!(
        out.status.code(),
        Some(2),
        "clap rejects unknown enum values"
    );
    let text = combined_output(&out);
    assert!(
        text.contains("--data-log-mode"),
        "parse error must name the flag:\n{text}"
    );
}

#[test]
fn vlm_refuse_existing_second_run_exits_2() {
    // The shared atomic refusal path, exercised through a second binary
    // (#305 review): the first run claims the path, the second is refused
    // before traffic.
    let mock = common::spawn_mock(&[]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("vlm-out.jsonl");
    let base = [
        "--url",
        &url,
        "--api-key",
        "dummy",
        "--scenario",
        "collision-vlm",
        "--num-requests",
        "1",
        "--concurrency",
        "1",
        "--prompts",
        "test-data/vlm/prompts.jsonl",
        "--model",
        "m",
        "--max-tokens",
        "8",
        "--log-level",
        "error",
    ];
    let mut first: Vec<String> = base.iter().map(|s| s.to_string()).collect();
    first.push("--data-log".into());
    first.push(data_log.to_str().unwrap().to_string());
    let status = Command::new(metrum_ai_bench_vlm_bin())
        .args(&first)
        .status()
        .expect("run metrum-ai-bench-cli-vlm");
    assert!(status.success());
    let before = std::fs::read_to_string(&data_log).unwrap();

    let mut second = first.clone();
    second.push("--data-log-mode".into());
    second.push("refuse-existing".into());
    let out = Command::new(metrum_ai_bench_vlm_bin())
        .args(&second)
        .output()
        .expect("run metrum-ai-bench-cli-vlm with refuse-existing");
    assert_eq!(
        out.status.code(),
        Some(2),
        "second vlm run must be refused with the configuration code"
    );
    let text = combined_output(&out);
    assert!(
        text.contains("already exists"),
        "refusal must name the collision:\n{text}"
    );
    assert_eq!(
        std::fs::read_to_string(&data_log).unwrap(),
        before,
        "refusal must not touch the existing log"
    );
}

// #307: standalone --version-only must print the version and exit 0 without
// traffic or required benchmark arguments, on every distributed binary.

#[test]
fn every_distributed_binary_supports_standalone_version_flags() {
    let with_version_only: Vec<(&str, PathBuf)> = vec![
        ("metrum-ai-bench-cli-llm", metrum_ai_bench_llm_bin()),
        ("metrum-ai-bench-cli-vlm", metrum_ai_bench_vlm_bin()),
        ("metrum-ai-bench-cli-asr", metrum_ai_bench_asr_bin()),
        (
            "metrum-ai-bench-cli-imagegen",
            metrum_ai_bench_imagegen_bin(),
        ),
        (
            "metrum-ai-bench-cli-strategic",
            metrum_ai_bench_strategic_bin(),
        ),
        ("metrum-ai-bench-cli-prompts", metrum_ai_bench_prompts_bin()),
    ];
    for (name, bin) in &with_version_only {
        // --version works everywhere (clap built-in).
        let out = Command::new(bin).arg("--version").output().expect(name);
        assert!(
            out.status.success(),
            "{name} --version must exit 0:\n{}",
            combined_output(&out)
        );
        // --version-only: the regression fixed by #307.
        let out = Command::new(bin)
            .arg("--version-only")
            .output()
            .expect(name);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{name} --version-only must print the version without required run arguments:\n{}",
            combined_output(&out)
        );
        let text = combined_output(&out);
        assert!(
            text.contains(&format!("{name} version")),
            "{name} --version-only must print the version line:\n{text}"
        );
    }
}

#[test]
fn dispatcher_binaries_have_no_version_only_flag() {
    // The unified dispatcher and the mock server never had --version-only;
    // --version covers them. This pins the current distribution surface so
    // the flag cannot silently appear or disappear.
    for bin in [
        PathBuf::from(env!("CARGO_BIN_EXE_metrum-ai-bench-cli")),
        PathBuf::from(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-mock-server")),
    ] {
        let out = Command::new(&bin).arg("--version").output().expect("bin");
        assert!(
            out.status.success(),
            "--version must work:\n{}",
            combined_output(&out)
        );
        let out = Command::new(&bin)
            .arg("--version-only")
            .output()
            .expect("bin");
        assert_eq!(
            out.status.code(),
            Some(2),
            "--version-only is not a flag of these binaries; parse must reject it:\n{}",
            combined_output(&out)
        );
    }
}

#[test]
fn llm_version_only_with_full_run_args_skips_traffic() {
    // --version-only wins even when a complete run is also configured: the
    // version prints and no traffic, data log, or banner side effects occur.
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args([
            "--version-only",
            "--url",
            "http://127.0.0.1:9/v1/chat/completions",
            "--api-key",
            "dummy",
            "--scenario",
            "t",
            "--num-requests",
            "1",
            "--concurrency",
            "1",
            "--prompts",
            "test-data/llm-hi.jsonl",
            "--mode",
            "chat",
            "--model",
            "m",
            "--data-log",
            data_log.to_str().unwrap(),
            "--max-tokens",
            "8",
        ])
        .output()
        .expect("run metrum-ai-bench-cli-llm --version-only with full args");
    assert!(
        out.status.success(),
        "--version-only must exit 0:\n{}",
        combined_output(&out)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("metrum-ai-bench-cli-llm version"),
        "version line must print:\n{stdout}"
    );
    assert!(
        !data_log.exists(),
        "no traffic or logging may occur with --version-only"
    );
}

// #310: tagged request rows must stream into the NDJSON log as requests
// complete, while issuance is still running — not only after the issuing
// loop finishes.

#[test]
fn ndjson_request_rows_stream_during_issuance() {
    use std::time::{Duration, Instant};

    // Long-latency server plus a paced (2 req/s) 40-request run: issuance
    // alone spans ~20s, so request rows observed below are provably emitted
    // mid-issuance.
    let mock = common::spawn_mock(&["--latency-ms", "300"]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let ndjson = dir.path().join("run.ndjson");
    let data_log = dir.path().join("run.jsonl");

    let mut args: Vec<String> = [
        "--url",
        &url,
        "--api-key",
        "dummy",
        "--scenario",
        "stream-ndjson",
        "--num-requests",
        "40",
        "--concurrency",
        "4",
        "--request-rate",
        "2",
        "--prompts",
        "test-data/llm-hi.jsonl",
        "--mode",
        "chat",
        "--model",
        "m",
        "--data-log",
        data_log.to_str().unwrap(),
        "--max-tokens",
        "8",
        "--log-level",
        "error",
        "--ndjson",
        ndjson.to_str().unwrap(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let _ = &mut args;
    let mut child = Command::new(metrum_ai_bench_llm_bin())
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn llm with --ndjson");

    // Tail the NDJSON log while the run is issuing: a request row must
    // appear before the process exits (i.e. before the final stage/summary).
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut observed_during_issuance = false;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("poll llm child") {
            assert!(
                observed_during_issuance,
                "run finished ({status}) before any request row was tailed"
            );
            break;
        }
        if let Ok(text) = std::fs::read_to_string(&ndjson) {
            if text.contains("\"kind\":\"request\"") {
                observed_during_issuance = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let _ = child.kill();
    let _ = child.wait();
    assert!(
        observed_during_issuance,
        "a tagged request row must appear in the NDJSON log while issuance is still running"
    );
}

#[test]
fn ndjson_request_rows_preserve_order_and_termination() {
    // After a full run: run row first, all request rows present exactly
    // once, summary row last (#310 keeps the ordered summary termination
    // and no duplicate/lost records when the consumer streams).
    let mock = common::spawn_mock(&["--latency-ms", "20"]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let ndjson = dir.path().join("run.ndjson");
    let data_log = dir.path().join("run.jsonl");
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args([
            "--url",
            &url,
            "--api-key",
            "dummy",
            "--scenario",
            "order-ndjson",
            "--num-requests",
            "12",
            "--concurrency",
            "4",
            "--prompts",
            "test-data/llm-hi.jsonl",
            "--mode",
            "chat",
            "--model",
            "m",
            "--data-log",
            data_log.to_str().unwrap(),
            "--max-tokens",
            "8",
            "--log-level",
            "error",
            "--ndjson",
            ndjson.to_str().unwrap(),
        ])
        .output()
        .expect("run llm with --ndjson");
    assert!(
        out.status.success(),
        "run must succeed:\n{}",
        combined_output(&out)
    );
    let text = std::fs::read_to_string(&ndjson).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines.first().unwrap().contains("\"kind\":\"run\""));
    assert!(lines.last().unwrap().contains("\"kind\":\"summary\""));
    let request_rows = lines
        .iter()
        .filter(|l| l.contains("\"kind\":\"request\""))
        .count();
    assert_eq!(request_rows, 12, "no lost or duplicated request rows");
    // Distinct seq values: 12 unique request rows.
    let seqs: std::collections::BTreeSet<u64> = lines
        .iter()
        .filter(|l| l.contains("\"kind\":\"request\""))
        .filter_map(|l| {
            serde_json::from_str::<serde_json::Value>(l)
                .ok()
                .and_then(|v| v.get("seq").and_then(serde_json::Value::as_u64))
        })
        .collect();
    assert_eq!(seqs.len(), 12, "each request row must have a unique seq");
}
// #303: agent/CI safety preset (--ci-safety implies --fail-on-error and
// --fail-on-partial). HTTP-500, all-error, endpoint-down and interrupted
// runs must not exit 0 under the preset; exploratory callers keep exit 0
// with logs and summaries written.

#[test]
fn ci_safety_preset_rejects_http_500_all_error_runs() {
    // Every mock response is a 503: an all-error run.
    let mock = common::spawn_mock(&["--fail-every", "1"]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");

    // Preset run: must fail.
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(collision_args(
            &url,
            data_log.to_str().unwrap(),
            &["--ci-safety"],
        ))
        .output()
        .expect("run all-error with --ci-safety");
    assert_eq!(
        out.status.code(),
        Some(1),
        "HTTP-500 all-error run must not pass the preset"
    );
    let text = combined_output(&out);
    assert!(
        text.contains("Test completed with errors"),
        "failure must name the errors:\n{text}"
    );

    // Exploratory default: same run, no preset, exit 0 with the summary
    // still written (compatibility preserved).
    let exploratory = dir.path().join("exploratory.jsonl");
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(collision_args(&url, exploratory.to_str().unwrap(), &[]))
        .output()
        .expect("run all-error without the preset");
    assert!(
        out.status.success(),
        "exploratory caller keeps exit 0:\n{}",
        combined_output(&out)
    );
    let logged = std::fs::read_to_string(&exploratory).unwrap();
    assert!(
        logged.contains("metrum-ai-bench-cli.summary.v3"),
        "summary must still be written for exploratory callers: {logged}"
    );
}

#[test]
fn ci_safety_preset_rejects_endpoint_down_runs() {
    // Valid absolute URL, nothing listening: endpoint-down run.
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(collision_args(
            "http://127.0.0.1:1/v1/chat/completions",
            data_log.to_str().unwrap(),
            &["--ci-safety"],
        ))
        .output()
        .expect("run endpoint-down with --ci-safety");
    assert_eq!(
        out.status.code(),
        Some(1),
        "endpoint-down run must not pass the preset"
    );
    let text = combined_output(&out);
    assert!(
        text.contains("Test completed with errors"),
        "failure must name the errors:\n{text}"
    );
}

#[test]
fn ci_safety_preset_rejects_interrupted_runs() {
    use std::time::{Duration, Instant};

    // Long-latency server so the run is still issuing when SIGINT lands.
    let mock = common::spawn_mock(&["--latency-ms", "400"]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");

    let mut interrupt_args = collision_args(&url, data_log.to_str().unwrap(), &["--ci-safety"]);
    // Replace the one-request defaults with a longer run (avoid duplicate
    // flags, which clap rejects with exit 2).
    for (flag, value) in [("--num-requests", "100"), ("--concurrency", "4")] {
        if let Some(pos) = interrupt_args.iter().position(|a| a == flag) {
            interrupt_args[pos + 1] = value.to_string();
        }
    }
    let mut child = Command::new(metrum_ai_bench_llm_bin())
        .args(&interrupt_args)
        .spawn()
        .expect("spawn llm with --ci-safety");

    // Let a few requests land, then interrupt.
    std::thread::sleep(Duration::from_millis(1200));
    let signalled = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .expect("send SIGINT")
        .success();
    assert!(signalled, "could not deliver SIGINT");

    let start = Instant::now();
    let status = loop {
        match child.try_wait().expect("wait") {
            Some(status) => break status,
            None if start.elapsed() > Duration::from_secs(60) => {
                let _ = child.kill();
                panic!("binary did not exit after SIGINT");
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    assert_eq!(
        status.code(),
        Some(1),
        "interrupted run must not pass the preset (got {status})"
    );

    // Logs and the partial summary are preserved even though the exit
    // verdict is a failure.
    let logged = std::fs::read_to_string(&data_log).expect("data log written");
    assert!(
        logged.contains("\"partial\":true"),
        "partial summary must be written:\n{logged}"
    );
    assert!(
        logged.lines().any(|l| l.contains("\"latency_s\"")),
        "completed request rows must be preserved:\n{logged}"
    );
}
// #308: strict SUT publication profile.

#[test]
fn llm_strict_sut_profile_rejects_missing_kernel_evidence_before_traffic() {
    let mock = common::spawn_mock(&[]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    // Publishable baseline but no kernel_evidence / quality_evidence.
    let sut = dir.path().join("sut.json");
    std::fs::write(
        &sut,
        std::fs::read_to_string("examples/sut.example.json").unwrap(),
    )
    .unwrap();

    let out = Command::new(metrum_ai_bench_llm_bin())
        .args([
            "--url",
            &url,
            "--api-key",
            "dummy",
            "--scenario",
            "strict-sut",
            "--num-requests",
            "1",
            "--concurrency",
            "1",
            "--prompts",
            "test-data/llm-hi.jsonl",
            "--mode",
            "chat",
            "--model",
            "m",
            "--data-log",
            data_log.to_str().unwrap(),
            "--max-tokens",
            "8",
            "--log-level",
            "error",
            "--sut",
            sut.to_str().unwrap(),
            "--sut-profile",
            "strict",
        ])
        .output()
        .expect("run llm with strict profile and incomplete evidence");
    assert!(
        !out.status.success(),
        "strict profile must refuse before traffic"
    );
    let text = combined_output(&out);
    assert!(
        text.contains("--sut-profile strict"),
        "error names the profile:\n{text}"
    );
    assert!(
        text.contains("model.kernel_evidence"),
        "error names the missing evidence:\n{text}"
    );
    assert!(
        text.contains("never infers kernel/graph choices from the model name"),
        "error states the no-inference rule:\n{text}"
    );
    assert!(
        !data_log.exists(),
        "refusal happens before any data-log write"
    );
}

#[test]
fn llm_strict_sut_profile_runs_with_complete_evidence() {
    let mock = common::spawn_mock(&[]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args([
            "--url",
            &url,
            "--api-key",
            "dummy",
            "--scenario",
            "strict-sut",
            "--num-requests",
            "1",
            "--concurrency",
            "1",
            "--prompts",
            "test-data/llm-hi.jsonl",
            "--mode",
            "chat",
            "--model",
            "m",
            "--data-log",
            data_log.to_str().unwrap(),
            "--max-tokens",
            "8",
            "--log-level",
            "error",
            "--sut",
            "examples/sut.strict.example.json",
            "--sut-profile",
            "strict",
        ])
        .output()
        .expect("run llm with strict profile and complete evidence");
    assert!(
        out.status.success(),
        "complete strict evidence runs:\n{}",
        combined_output(&out)
    );
    // #327 review: the #142 warning must still fire in strict mode — the
    // strict profile gates publication, it does not replace the warning.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("performance, not answer quality"),
        "quantization warning fires in strict mode too:\n{stderr}"
    );
    let logged = std::fs::read_to_string(&data_log).unwrap();
    assert!(
        logged.contains("\"kv_dtype\":\"fp8_e4m3\""),
        "strict evidence lands in the summary sut block: {logged}"
    );
    assert!(
        logged.contains("\"quality_evidence\""),
        "quality evidence link stays in the summary, separate from throughput: {logged}"
    );
}
// #306: Endpoints-compatible, explicitly non-certifying artifact adapter.

fn unified_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_metrum-ai-bench-cli"))
}

#[test]
fn endpoints_export_builds_parseable_bundle_from_a_real_run() {
    let mock = common::spawn_mock(&[]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("run.jsonl");
    let status = Command::new(metrum_ai_bench_llm_bin())
        .args([
            "--url",
            &url,
            "--api-key",
            "dummy",
            "--scenario",
            "adapter-e2e",
            "--num-requests",
            "4",
            "--concurrency",
            "2",
            "--prompts",
            "test-data/llm-hi.jsonl",
            "--mode",
            "chat",
            "--model",
            "example/model",
            "--data-log",
            data_log.to_str().unwrap(),
            "--max-tokens",
            "8",
            "--log-level",
            "error",
        ])
        .status()
        .expect("run llm");
    assert!(status.success());

    let out_dir = dir.path().join("bundle");
    let out = Command::new(unified_bin())
        .args([
            "endpoints-export",
            "--data-log",
            data_log.to_str().unwrap(),
            "--out-dir",
            out_dir.to_str().unwrap(),
            "--workload",
            "W1",
            "--scenario",
            "server",
        ])
        .output()
        .expect("run endpoints-export");
    assert!(
        out.status.success(),
        "adapter must succeed:\n{}",
        combined_output(&out)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("EXPLICITLY NON-CERTIFYING"),
        "stdout prints the disclaimer:\n{stdout}"
    );

    // The bundle is upstream-parser-shaped: performance/result_summary.json
    // carries the keys the endpoints_parser reads.
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(out_dir.join("performance/result_summary.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(result["n_samples_issued"], 4);
    assert_eq!(result["n_samples_completed"], 4);
    assert_eq!(result["n_samples_failed"], 0);
    assert!(result["qps"].as_f64().unwrap() > 0.0);
    assert!(result["duration_ns"].as_u64().unwrap() > 0);
    assert!(
        result["result_completed_tokens_per_second"]
            .as_f64()
            .unwrap()
            > 0.0
    );

    // Non-certifying files always present.
    let compliance = std::fs::read_to_string(out_dir.join("NONCOMPLIANCE.md")).unwrap();
    assert!(compliance.contains("IMPOSSIBLE"));
    let power: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out_dir.join("system_power.json")).unwrap())
            .unwrap();
    assert_eq!(power["declared"], false, "no synthesized power facts");
}

#[test]
fn endpoints_export_refuses_partial_runs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("run.jsonl");
    std::fs::write(
        &data_log,
        concat!(
            "{\"schema_version\":\"metrum-ai-bench-cli.request.v1\",\"run_id\":\"r\",\"seq\":0,\"phase\":\"measure\",\"started_at\":\"2026-01-01T00:00:00Z\",\"completed_at\":\"2026-01-01T00:00:01Z\",\"latency_s\":1.0,\"ttft_s\":0.1,\"prompt_tokens\":1,\"completion_tokens\":2,\"usage_missing\":false,\"error\":null}\n",
            "{\"schema_version\":\"metrum-ai-bench-cli.summary.v3\",\"run_id\":\"r\",\"partial\":true,\"window_seconds\":1.0}\n"
        ),
    )
    .unwrap();
    let out = Command::new(unified_bin())
        .args([
            "endpoints-export",
            "--data-log",
            data_log.to_str().unwrap(),
            "--out-dir",
            dir.path().join("bundle").to_str().unwrap(),
            "--workload",
            "W1",
            "--scenario",
            "server",
        ])
        .output()
        .expect("run endpoints-export on a partial log");
    assert!(!out.status.success(), "partial runs are not exportable");
    let text = combined_output(&out);
    assert!(text.contains("partial"), "error names the cause:\n{text}");
}

/// #306: pinned external-checker integration. Skipped unless the operator
/// provides a checkout of the upstream submission checker (the study's
/// pinned commit fda43ef8… is not publicly fetchable; see NONCOMPLIANCE.md).
/// Set MLCOMMONS_INFERENCE_CHECKOUT=/path/to/mlcommons/inference to run.
#[test]
fn endpoints_export_upstream_parser_smoke() {
    let Some(checkout) = std::env::var_os("MLCOMMONS_INFERENCE_CHECKOUT") else {
        eprintln!("skipped: MLCOMMONS_INFERENCE_CHECKOUT not set");
        return;
    };
    let parser = PathBuf::from(checkout)
        .join("tools/submission/submission_checker/parsers/endpoints_parser.py");
    if !parser.exists() {
        eprintln!("skipped: endpoints parser not present in the checkout");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    // Build a bundle via the library (golden fixture).
    let log = dir.path().join("run.jsonl");
    std::fs::write(
        &log,
        concat!(
            "{\"schema_version\":\"metrum-ai-bench-cli.request.v1\",\"run_id\":\"r\",\"seq\":0,\"phase\":\"measure\",\"started_at\":\"2026-01-01T00:00:00Z\",\"completed_at\":\"2026-01-01T00:00:01Z\",\"latency_s\":1.0,\"ttft_s\":0.1,\"prompt_tokens\":4,\"completion_tokens\":8,\"usage_missing\":false,\"error\":null}\n",
            "{\"schema_version\":\"metrum-ai-bench-cli.summary.v3\",\"run_id\":\"r\",\"partial\":false,\"window_seconds\":1.0,\"ttft_s\":{\"p50\":0.1,\"p90\":0.1},\"completion_tokens_per_second\":8.0,\"config\":{\"model\":\"m\",\"concurrency\":1,\"arrival\":\"closed-loop\"}}\n"
        ),
    )
    .unwrap();
    let options = metrum_ai_bench::endpoints_export::ExportOptions {
        data_log: log,
        out_dir: dir.path().join("bundle"),
        sut: None,
        workload: "W1".into(),
        scenario: metrum_ai_bench::endpoints_export::ScenarioKind::Server,
        power_desc: None,
        notes: None,
    };
    metrum_ai_bench::endpoints_export::export_bundle(&options).unwrap();
    let status = Command::new("python3")
        .args([
            "-c",
            "import sys; sys.path.insert(0, sys.argv[1]); from tools.submission.submission_checker.parsers.endpoints_parser import EndpointsParser; p = EndpointsParser(sys.argv[2]); p[\"qps\"]; p[\"n_samples_issued\"]; print(\"parser-ok\")",
            parser
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .to_str()
                .unwrap(),
            dir.path().join("bundle").to_str().unwrap(),
        ])
        .output()
        .expect("run upstream parser");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&status.stdout),
        String::from_utf8_lossy(&status.stderr)
    );
    assert!(
        status.status.success() && text.contains("parser-ok"),
        "upstream endpoints_parser must parse the bundle:\n{text}"
    );
}
// #309: native Offline-style burst mode (--arrival burst).

fn burst_args(url: &str, data_log: &str, extra: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = [
        "--url",
        url,
        "--api-key",
        "dummy",
        "--scenario",
        "burst",
        "--num-requests",
        "8",
        "--concurrency",
        "1",
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
fn burst_issues_whole_population_at_once_and_drains_into_the_window() {
    // Issuance timeline + duration denominator (#309): 8 requests at
    // --concurrency 1 (ignored by burst) with 100 ms server latency would
    // take ~800 ms sequentially; burst issues them all at t≈0, so the
    // window runs from the first send to the LAST completion (tail drain
    // included) and ends near the server latency, not the serial sum.
    let mock = common::spawn_mock(&["--latency-ms", "100"]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(burst_args(
            &url,
            data_log.to_str().unwrap(),
            &["--arrival", "burst"],
        ))
        .output()
        .expect("run burst");
    assert!(
        out.status.success(),
        "burst run must succeed:\n{}",
        combined_output(&out)
    );
    let text = std::fs::read_to_string(&data_log).unwrap();
    let rows: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let requests: Vec<&serde_json::Value> = rows
        .iter()
        .filter(|r| {
            r["schema_version"]
                .as_str()
                .unwrap_or("")
                .contains(".request.")
        })
        .collect();
    assert_eq!(requests.len(), 8, "all requests recorded");

    // Issuance timeline: every request sent within a small window of the
    // run start (send_offset_s is epoch-based and recorded for burst).
    let first_send = requests
        .iter()
        .filter_map(|r| r["send_offset_s"].as_f64())
        .fold(f64::MAX, f64::min);
    for r in &requests {
        let send = r["send_offset_s"]
            .as_f64()
            .expect("burst records send_offset_s");
        assert!(
            send - first_send < 0.5,
            "burst sends are immediate: send_offset {send}s"
        );
    }

    // Duration denominator: the summary window runs from the first send to
    // the last completion — the tail drain is inside it.
    let summary = rows.last().unwrap();
    let window = summary["window_seconds"].as_f64().unwrap();
    assert!(
        (0.09..0.6).contains(&window),
        "window {window}s: includes the 100ms server latency (tail drain), excludes the serial sum (~800ms)"
    );

    // Reproducible request shape: the burst is stamped as its own arrival,
    // and the effective transport cap is the whole population.
    let arrival = summary["config"]["common"]["arrival"]
        .as_str()
        .unwrap_or("");
    assert_eq!(arrival, "burst", "summary stamps the burst arrival");
    assert_eq!(
        summary["config"]["effective_max_concurrency"].as_u64(),
        Some(8),
        "uncapped burst issues the whole population"
    );
}

#[test]
fn burst_respects_explicit_transport_cap_and_records_queue_wait() {
    // Transport/connection limits (#309): --max-concurrency is the explicit
    // cap; waits beyond it are recorded as queue delay, not hidden.
    let mock = common::spawn_mock(&["--latency-ms", "200"]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(burst_args(
            &url,
            data_log.to_str().unwrap(),
            &["--arrival", "burst", "--max-concurrency", "2"],
        ))
        .output()
        .expect("run capped burst");
    assert!(out.status.success(), "capped burst must succeed");
    let text = std::fs::read_to_string(&data_log).unwrap();
    let requests: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .filter(|r| {
            r["schema_version"]
                .as_str()
                .unwrap_or("")
                .contains(".request.")
        })
        .collect();
    let rows = requests.clone();
    let waited = rows
        .iter()
        .filter(|r| r["queue_delay_s"].as_f64().unwrap_or(0.0) > 0.15)
        .count();
    assert!(
        waited >= 4,
        "with cap 2, the later waves wait visibly: {waited} of {} rows",
        rows.len()
    );
}

#[test]
fn burst_rejects_request_rate_combination() {
    let mock = common::spawn_mock(&[]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(burst_args(
            &url,
            data_log.to_str().unwrap(),
            &["--arrival", "burst", "--request-rate", "5"],
        ))
        .output()
        .expect("run burst with rate");
    assert!(
        !out.status.success(),
        "burst + request-rate is contradictory"
    );
    let text = combined_output(&out);
    assert!(
        text.contains("--request-rate"),
        "error must name the conflicting flag:\n{text}"
    );
}

#[test]
fn burst_error_handling_counts_failures_and_still_drains() {
    // Error handling (#309): with every 2nd request failing, the run still
    // drains the full batch and the failure counts are exact.
    let mock = common::spawn_mock(&["--latency-ms", "50", "--fail-every", "2"]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(burst_args(
            &url,
            data_log.to_str().unwrap(),
            &["--arrival", "burst"],
        ))
        .output()
        .expect("run burst with failures");
    assert!(
        out.status.success(),
        "default exit semantics keep partial success:\n{}",
        combined_output(&out)
    );
    let text = std::fs::read_to_string(&data_log).unwrap();
    let rows: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let requests: Vec<&serde_json::Value> = rows
        .iter()
        .filter(|r| {
            r["schema_version"]
                .as_str()
                .unwrap_or("")
                .contains(".request.")
        })
        .collect();
    let failed = requests.iter().filter(|r| !r["error"].is_null()).count();
    assert_eq!(failed, 4, "every 2nd of 8 failed: {rows:?}");
    assert_eq!(requests.len(), 8, "every request is recorded");
    let summary = rows.last().unwrap();
    assert_eq!(summary["attempted"], 8);
    assert!(summary["errors"].as_u64().unwrap() >= 4);
}

// #309 review: uncapped-burst fd guard and cap-source stamping.

#[test]
fn burst_uncapped_stamps_cap_source_and_population_cap() {
    let mock = common::spawn_mock(&["--latency-ms", "50"]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(burst_args(
            &url,
            data_log.to_str().unwrap(),
            &["--arrival", "burst"],
        ))
        .output()
        .expect("run uncapped burst");
    assert!(out.status.success());
    let text = std::fs::read_to_string(&data_log).unwrap();
    let summary: serde_json::Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    assert_eq!(
        summary["config"]["effective_max_concurrency_source"],
        "uncapped-burst"
    );
    assert_eq!(
        summary["config"]["effective_max_concurrency"].as_u64(),
        Some(8),
        "uncapped burst caps at the population"
    );
}

#[test]
fn burst_with_explicit_cap_stamps_the_cap() {
    let mock = common::spawn_mock(&["--latency-ms", "50"]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(burst_args(
            &url,
            data_log.to_str().unwrap(),
            &["--arrival", "burst", "--max-concurrency", "3"],
        ))
        .output()
        .expect("run capped burst");
    assert!(out.status.success());
    let text = std::fs::read_to_string(&data_log).unwrap();
    let summary: serde_json::Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    assert_eq!(
        summary["config"]["effective_max_concurrency_source"],
        "max-concurrency"
    );
    assert_eq!(
        summary["config"]["effective_max_concurrency"].as_u64(),
        Some(3)
    );
}

#[test]
fn uncapped_burst_over_fd_budget_exits_2_before_traffic() {
    // A population beyond the fd budget must be refused with the
    // configuration error code, before any traffic (#309 review).
    let mock = common::spawn_mock(&[]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let budget = metrum_ai_bench::runner::fd_soft_limit().saturating_sub(128) + 1;
    let mut args = burst_args(&url, data_log.to_str().unwrap(), &["--arrival", "burst"]);
    if let Some(pos) = args.iter().position(|a| a == "8") {
        args[pos] = budget.to_string();
    }
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(&args)
        .output()
        .expect("run oversize burst");
    assert_eq!(
        out.status.code(),
        Some(2),
        "oversize uncapped burst exits 2"
    );
    let text = combined_output(&out);
    assert!(
        text.contains("--max-concurrency"),
        "message names the fix:\n{text}"
    );
    assert!(!data_log.exists(), "no traffic before refusal");
}

#[test]
fn non_burst_stamp_is_unchanged() {
    // #309 review: pin the existing effective_max_concurrency stamp for
    // non-burst runs — only burst runs may change it.
    let mock = common::spawn_mock(&["--latency-ms", "20"]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    // Closed loop, --concurrency 2, no --max-concurrency.
    let mut closed_args = burst_args(&url, data_log.to_str().unwrap(), &[]);
    // Replace the baked-in counts (avoid duplicate flags: clap rejects them).
    for (flag, value) in [("--num-requests", "4"), ("--concurrency", "2")] {
        let pos = closed_args.iter().position(|a| a == flag).unwrap();
        closed_args[pos + 1] = value.to_string();
    }
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(&closed_args)
        .output()
        .expect("run closed loop");
    assert!(out.status.success());
    let text = std::fs::read_to_string(&data_log).unwrap();
    let summary: serde_json::Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    assert_eq!(
        summary["config"]["effective_max_concurrency"].as_u64(),
        Some(2),
        "closed-loop stamps the --concurrency value"
    );
    assert_eq!(
        summary["config"]["effective_max_concurrency_source"],
        "concurrency"
    );

    // Open-loop poisson with an explicit cap stamps the cap.
    let data_log2 = dir.path().join("out2.jsonl");
    let mut open_args = burst_args(
        &url,
        data_log2.to_str().unwrap(),
        &[
            "--request-rate",
            "1000",
            "--arrival",
            "poisson",
            "--max-concurrency",
            "4",
        ],
    );
    for (flag, value) in [("--num-requests", "6"), ("--concurrency", "2")] {
        let pos = open_args.iter().position(|a| a == flag).unwrap();
        open_args[pos + 1] = value.to_string();
    }
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(&open_args)
        .output()
        .expect("run open loop");
    assert!(out.status.success());
    let text = std::fs::read_to_string(&data_log2).unwrap();
    let summary: serde_json::Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    assert_eq!(
        summary["config"]["effective_max_concurrency"].as_u64(),
        Some(4),
        "open-loop stamps the --max-concurrency value"
    );
    assert_eq!(
        summary["config"]["effective_max_concurrency_source"],
        "max-concurrency"
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

// Integration follow-up (a) (#320×#321): the --validate-model preflight runs
// BEFORE prepare_data_log, so a failed overwrite-mode preflight never
// destroys an existing log.

#[test]
fn overwrite_mode_with_failed_model_preflight_leaves_old_log_untouched() {
    let mock = common::spawn_mock(&[]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    // Seed an existing log with a prior complete run.
    let seeded = dir.path().join("seed.jsonl");
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(collision_args(&url, seeded.to_str().unwrap(), &[]))
        .output()
        .expect("seed run");
    assert!(out.status.success());
    std::fs::copy(&seeded, &data_log).unwrap();
    let before = std::fs::read_to_string(&data_log).unwrap();

    // overwrite + --validate-model with an unknown model must exit 2 and
    // leave the old log byte-identical.
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(collision_args(
            &url,
            data_log.to_str().unwrap(),
            &["--data-log-mode", "overwrite", "--validate-model"],
        ))
        .output()
        .expect("run overwrite with failed preflight");
    assert_eq!(
        out.status.code(),
        Some(2),
        "failed preflight exits 2 (configuration)"
    );
    let text = combined_output(&out);
    assert!(
        text.contains("model preflight failed"),
        "preflight failure named:\n{text}"
    );
    assert_eq!(
        std::fs::read_to_string(&data_log).unwrap(),
        before,
        "the old log is byte-identical — preflight ran before overwrite truncation"
    );
}

// Integration follow-up (b) (#323×#321): --ci-safety defaults --data-log-mode
// to refuse-existing; an explicit --data-log-mode wins.

#[test]
fn ci_safety_defaults_data_log_mode_to_refuse_existing() {
    let mock = common::spawn_mock(&[]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    // Seed an existing log.
    let seeded = dir.path().join("seed.jsonl");
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(collision_args(&url, seeded.to_str().unwrap(), &[]))
        .output()
        .expect("seed run");
    assert!(out.status.success());
    std::fs::copy(&seeded, &data_log).unwrap();

    // --ci-safety with no explicit mode: refuse-existing, exit 2, no traffic.
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(collision_args(
            &url,
            data_log.to_str().unwrap(),
            &["--ci-safety"],
        ))
        .output()
        .expect("run ci-safety against an existing log");
    assert_eq!(
        out.status.code(),
        Some(2),
        "preset defaults to refuse-existing"
    );
    let text = combined_output(&out);
    assert!(text.contains("already exists"), "refusal named:\n{text}");

    // Explicit append wins over the preset default.
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(collision_args(
            &url,
            data_log.to_str().unwrap(),
            &["--ci-safety", "--data-log-mode", "append"],
        ))
        .output()
        .expect("run ci-safety with explicit append");
    assert!(
        out.status.success(),
        "explicit append wins:\n{}",
        combined_output(&out)
    );
    let logged = std::fs::read_to_string(&data_log).unwrap();
    let summaries = logged
        .lines()
        .filter(|l| l.contains("metrum-ai-bench-cli.summary.v3"))
        .count();
    assert_eq!(summaries, 2, "the second run appended: {logged}");
}

// Integration follow-up (c) (#329×#330): endpoints-export --scenario offline
// on a burst run discloses native burst, not a closed-loop proxy.

#[test]
fn endpoints_export_offline_on_burst_says_native_burst() {
    let mock = common::spawn_mock(&["--latency-ms", "50"]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(burst_args(
            &url,
            data_log.to_str().unwrap(),
            &["--arrival", "burst"],
        ))
        .output()
        .expect("burst run");
    assert!(out.status.success());

    let out_dir = dir.path().join("bundle");
    let out = Command::new(unified_bin())
        .args([
            "endpoints-export",
            "--data-log",
            data_log.to_str().unwrap(),
            "--out-dir",
            out_dir.to_str().unwrap(),
            "--workload",
            "W1",
            "--scenario",
            "offline",
        ])
        .output()
        .expect("export burst run as offline");
    assert!(out.status.success(), "{}", combined_output(&out));

    let point = std::fs::read_to_string(out_dir.join("point.yaml")).unwrap();
    assert!(
        point.contains("native burst (bench definition; not an accepted MLPerf Offline result)"),
        "burst runs disclose native burst, not a proxy:\n{point}"
    );
    assert!(
        !point.contains("closed-loop/fixed-rate run"),
        "the closed-loop proxy disclosure must not fire for burst runs:\n{point}"
    );
}

// Integration follow-up (d) (#331×#321): the SSE trace path derives from the
// effective data log (new-dir → <run_id>.sse-trace.jsonl) and follows the
// collision policy.

#[test]
fn sse_trace_under_new_dir_is_per_run_not_shared() {
    let mock = common::spawn_mock(&["--latency-ms", "30"]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let runs_dir = dir.path().join("runs");
    for _ in 0..2 {
        let out = Command::new(metrum_ai_bench_llm_bin())
            .args(seq_run_args(
                &url,
                runs_dir.to_str().unwrap(),
                &[
                    "--data-log-mode",
                    "new-dir",
                    "--streaming",
                    "--sse-trace-first-n",
                    "1",
                ],
            ))
            .output()
            .expect("run new-dir with trace");
        assert!(out.status.success(), "{}", combined_output(&out));
    }
    let entries: Vec<_> = std::fs::read_dir(&runs_dir).unwrap().collect();
    let trace_files: Vec<_> = entries
        .iter()
        .map(|e| e.as_ref().unwrap().path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".sse-trace.jsonl"))
        })
        .collect();
    assert_eq!(
        trace_files.len(),
        2,
        "one per-run trace file per run: {trace_files:?}"
    );
    // Each is named <run_id>.sse-trace.jsonl, not a shared sibling.
    for p in &trace_files {
        let name = p.file_name().unwrap().to_str().unwrap();
        assert_eq!(
            name.split('.').next().unwrap().len(),
            36,
            "per-run uuid stem (36-char run id): {name}"
        );
    }
}

#[test]
fn sse_trace_refuses_when_trace_file_exists() {
    let mock = common::spawn_mock(&[]);
    let url = format!("http://{}/v1/chat/completions", mock.address);
    let dir = tempfile::tempdir().expect("tempdir");
    let data_log = dir.path().join("out.jsonl");
    let trace = format!("{}.sse-trace.jsonl", data_log.to_str().unwrap());
    std::fs::write(&trace, "prior trace\n").unwrap();
    // Seed the data log too so the data-log refusal would also fire — use a
    // fresh data log to isolate the trace refusal.
    let fresh = dir.path().join("fresh.jsonl");
    // sibling_trace_path strips .jsonl then appends .sse-trace.jsonl.
    let fresh_trace = format!(
        "{}.sse-trace.jsonl",
        fresh.to_str().unwrap().trim_end_matches(".jsonl")
    );
    std::fs::write(&fresh_trace, "prior trace\n").unwrap();
    let out = Command::new(metrum_ai_bench_llm_bin())
        .args(seq_run_args(
            &url,
            fresh.to_str().unwrap(),
            &[
                "--data-log-mode",
                "refuse-existing",
                "--streaming",
                "--sse-trace-first-n",
                "1",
            ],
        ))
        .output()
        .expect("run refuse-existing with existing trace");
    assert_eq!(
        out.status.code(),
        Some(2),
        "the trace file follows the collision policy"
    );
    let text = combined_output(&out);
    assert!(
        text.contains("SSE trace file collision"),
        "the refusal names the trace file:\n{text}"
    );
}
