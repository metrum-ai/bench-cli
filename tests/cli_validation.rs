// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Integration tests: CLI validation and fail-fast behavior (issue #128).
//! Run with: cargo test --test cli_validation

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

// Cargo sets CARGO_BIN_EXE_<name> (exact binary name, case preserved) at
// compile time for integration tests, so the binaries are found regardless of
// CARGO_TARGET_DIR. The previous lookup uppercased the name and silently fell
// back to target/debug, which only exists in the default layout.
fn metrum_ai_bench_llm_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_metrum-ai-bench-llm"))
}

fn metrum_ai_bench_vlm_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_metrum-ai-bench-vlm"))
}

fn metrum_ai_bench_asr_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_metrum-ai-bench-asr"))
}

fn metrum_ai_bench_prompts_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_metrum-ai-bench-prompts"))
}

fn metrum_ai_bench_imagegen_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_metrum-ai-bench-imagegen"))
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
        .expect("run metrum-ai-bench-llm");
    assert!(
        !out.status.success(),
        "metrum-ai-bench-llm --url without --api-key must fail at parse time"
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
        .expect("run metrum-ai-bench-vlm");
    assert!(
        !out.status.success(),
        "metrum-ai-bench-vlm --url without --api-key must fail at parse time"
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
        .expect("run metrum-ai-bench-asr");
    assert!(
        !out.status.success(),
        "metrum-ai-bench-asr --url without --api-key must fail at parse time"
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
        .expect("run metrum-ai-bench-imagegen");
    assert!(
        !out.status.success(),
        "metrum-ai-bench-imagegen --url without --api-key must fail at parse time"
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
        .expect("run metrum-ai-bench-llm with endpoints-file");
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
        .expect("run metrum-ai-bench-llm --quiet");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("@@@"),
        "quiet must suppress ASCII art:\n{stdout}"
    );
    assert!(
        stdout
            .lines()
            .any(|l| l.starts_with("Metrum AI Bench metrum-ai-bench-llm ")),
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
        .expect("run metrum-ai-bench-asr --version-only");

    assert!(
        out.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("metrum-ai-bench-asr version"),
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
        .expect("run metrum-ai-bench-llm");
    assert!(
        !out.status.success(),
        "metrum-ai-bench-llm must reject --concurrency 0"
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
        .expect("run metrum-ai-bench-llm");
    assert!(
        !out.status.success(),
        "metrum-ai-bench-llm must reject invalid --mode"
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
        .expect("run metrum-ai-bench-llm");
    assert!(
        !out.status.success(),
        "metrum-ai-bench-llm must reject --num-requests 0"
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
        .expect("run metrum-ai-bench-vlm");
    assert!(
        !out.status.success(),
        "metrum-ai-bench-vlm must reject --concurrency 0"
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
        .expect("run metrum-ai-bench-vlm");
    assert!(
        !out.status.success(),
        "metrum-ai-bench-vlm must reject --image-cache-size 0"
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
        .expect("run metrum-ai-bench-vlm");
    assert!(
        !out.status.success(),
        "metrum-ai-bench-vlm must reject invalid --image-detail"
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
        .expect("run metrum-ai-bench-asr");
    assert!(
        !out.status.success(),
        "metrum-ai-bench-asr must reject --concurrency 0"
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
        .expect("run metrum-ai-bench-asr");
    assert!(
        !out.status.success(),
        "metrum-ai-bench-asr must reject --num-requests 0"
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
        .expect("run metrum-ai-bench-asr");
    assert!(
        !out.status.success(),
        "metrum-ai-bench-asr must reject invalid --response-format"
    );
}

#[test]
fn metrum_ai_bench_prompts_version_only() {
    let out = Command::new(metrum_ai_bench_prompts_bin())
        .args(["--version-only"])
        .output()
        .expect("run metrum-ai-bench-prompts --version-only");
    assert!(
        out.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("metrum-ai-bench-prompts version"));
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
        .expect("run metrum-ai-bench-prompts");
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
        .expect("run metrum-ai-bench-prompts");
    assert!(!out.status.success());
    assert!(!output.exists(), "must not write JSONL on failure");
    assert!(!report.exists(), "must not write report on failure");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no mix within ISL/OSL") || stderr.contains("kind:"),
        "stderr:\n{stderr}"
    );
}
