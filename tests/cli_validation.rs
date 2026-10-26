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
    assert!(
        !out.status.success(),
        "scheme-less --url must fail the run, not benchmark a broken endpoint"
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
    assert!(!out.status.success(), "must fail fast");
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
    assert!(!out.status.success(), "must fail fast");
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
    assert!(!out.status.success(), "must fail fast");
    let text = combined_output(&out);
    assert!(
        text.contains("invalid endpoint URL"),
        "error must explain the invalid URL:\n{text}"
    );
}

// #304: opt-in exact-model preflight (--validate-model), tested against the
// workspace mock server (serves /v1/models with "metrum-ai-bench-cli-mock").

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
    assert!(!out.status.success(), "unknown model must fail preflight");
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
