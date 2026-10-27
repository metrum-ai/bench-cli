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
