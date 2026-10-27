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

/// Full metrum-ai-bench-cli-llm argument set for a one-request run against
/// `url`, writing `data_log`, with `extra` args appended.
fn collision_args(url: &str, data_log: &str, extra: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = [
        "--url",
        url,
        "--api-key",
        "dummy",
        "--scenario",
        "ci-safety",
        "--num-requests",
        "2",
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
