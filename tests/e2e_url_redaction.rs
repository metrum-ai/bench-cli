// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Endpoint URL credential redaction in per-request logs (#332).
//!
//! A failing request against a URL with userinfo and query credentials
//! (`?api_key=` / `?token=`) must never echo the secret parts in
//! `error.log`, `debug.log` or stderr. Every modality's request-failure
//! logging goes through `metrum_ai_bench::endpoints::redact_url` /
//! `redact_reqwest_error`, so the logs stay shippable.
//!
//! No dummy-model-server needed: the endpoint points at a closed local
//! port, so each request fails fast with a connect error before any
//! network traffic leaves the host.

use std::io::Write;
use std::process::Command;

/// Secret fragments that must never appear in any log; chosen so that a
/// partial redaction cannot accidentally pass (each is one token).
const SECRETS: [&str; 3] = ["sekrit-pw", "sekrit-key", "sekrit-tok"];

/// Unreachable endpoint whose URL carries userinfo and query credentials.
fn credentialed_url(path: &str) -> String {
    format!("http://user:sekrit-pw@127.0.0.1:1{path}?api_key=sekrit-key&token=sekrit-tok")
}

struct Fixture {
    _dir: tempfile::TempDir,
    prompts: std::path::PathBuf,
    data_log: std::path::PathBuf,
    debug_log: std::path::PathBuf,
    error_log: std::path::PathBuf,
}

fn fixture() -> Fixture {
    fixture_with_prompt_row(r#"{"prompt":"Hi"}"#)
}

fn fixture_with_prompt_row(row: &str) -> Fixture {
    let dir = tempfile::tempdir().expect("tmpdir");
    let prompts = dir.path().join("prompts.jsonl");
    let mut file = std::fs::File::create(&prompts).expect("create prompts");
    writeln!(file, "{row}").expect("write prompt");
    Fixture {
        prompts,
        data_log: dir.path().join("out.jsonl"),
        debug_log: dir.path().join("debug.log"),
        error_log: dir.path().join("error.log"),
        _dir: dir,
    }
}

/// Assert none of the secret fragments appears in `error.log`, `debug.log`
/// or the captured process output (stderr and stdout).
fn assert_no_secrets(fixture: &Fixture, output: &std::process::Output, what: &str) {
    for (label, text) in [
        (
            "error.log",
            std::fs::read_to_string(&fixture.error_log).unwrap_or_default(),
        ),
        (
            "debug.log",
            std::fs::read_to_string(&fixture.debug_log).unwrap_or_default(),
        ),
        (
            "stderr",
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ),
        (
            "stdout",
            String::from_utf8_lossy(&output.stdout).into_owned(),
        ),
    ] {
        for secret in SECRETS {
            assert!(
                !text.contains(secret),
                "{what}: {label} leaks credential '{secret}':\n{text}"
            );
        }
    }
}

/// Positive control: the URL lines must still be present, redacted, so the
/// absence assertions cannot pass because the logs were empty.
fn assert_redacted_marker_present(fixture: &Fixture, what: &str) {
    let debug_log = std::fs::read_to_string(&fixture.debug_log).unwrap_or_default();
    let error_log = std::fs::read_to_string(&fixture.error_log).unwrap_or_default();
    assert!(
        debug_log.contains("api_key=REDACTED") || error_log.contains("api_key=REDACTED"),
        "{what}: no redacted URL marker in the logs; the fixture never exercised a URL log line\ndebug.log:\n{debug_log}\nerror.log:\n{error_log}"
    );
}

/// The llm binary must not echo credentialed URLs in its request-failure
/// logs: the streaming branch's reqwest report and context block, the
/// non-streaming branch, and the per-request `Request failed` line.
#[test]
fn llm_failure_logs_do_not_leak_url_credentials() {
    for extra in [&["--streaming"][..], &[][..]] {
        let fixture = fixture();
        let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-llm"))
            .args([
                "--url",
                &credentialed_url("/v1/chat/completions"),
                "--api-key",
                "dummy",
                "--scenario",
                "e2e-url-redaction",
                "--num-requests",
                "1",
                "--concurrency",
                "1",
                "--prompts",
                fixture.prompts.to_str().unwrap(),
                "--mode",
                "chat",
                "--model",
                "dummy",
                "--max-tokens",
                "10",
                "--connect-timeout",
                "2",
                "--data-log",
                fixture.data_log.to_str().unwrap(),
                "--debug-log",
                fixture.debug_log.to_str().unwrap(),
                "--error-log",
                fixture.error_log.to_str().unwrap(),
                // debug level exercises the per-request debug URL lines too.
                "--log-level",
                "debug",
            ])
            .args(extra)
            .output()
            .expect("run llm");
        assert_no_secrets(
            &fixture,
            &output,
            &format!("llm (streaming={})", extra.first() == Some(&"--streaming")),
        );
        assert_redacted_marker_present(
            &fixture,
            &format!("llm (streaming={})", extra.first() == Some(&"--streaming")),
        );
        // The request must actually fail (connect error), not silently
        // succeed, or the fixture proves nothing.
        let data_log = std::fs::read_to_string(&fixture.data_log).unwrap_or_default();
        assert!(
            data_log.contains("connect") || data_log.contains("timeout"),
            "fixture must record a failed request, data log:\n{data_log}"
        );
    }
}

/// Same contract for the vlm binary, including the `image_url` fetch path
/// in the library (its debug line and error messages embed that URL too).
#[test]
fn vlm_failure_logs_do_not_leak_url_credentials() {
    let image_url = credentialed_url("/img.png");
    let fixture = fixture_with_prompt_row(&format!(
        r#"{{"prompt":"Describe this image","image_url":"{image_url}"}}"#
    ));
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-vlm"))
        .args([
            "--url",
            &credentialed_url("/v1/chat/completions"),
            "--api-key",
            "dummy",
            "--scenario",
            "e2e-url-redaction",
            "--num-requests",
            "1",
            "--concurrency",
            "1",
            "--prompts",
            fixture.prompts.to_str().unwrap(),
            "--model",
            "dummy",
            "--max-tokens",
            "10",
            "--connect-timeout",
            "2",
            "--data-log",
            fixture.data_log.to_str().unwrap(),
            "--debug-log",
            fixture.debug_log.to_str().unwrap(),
            "--error-log",
            fixture.error_log.to_str().unwrap(),
            "--log-level",
            "debug",
        ])
        .output()
        .expect("run vlm");
    assert_no_secrets(&fixture, &output, "vlm");
    assert_redacted_marker_present(&fixture, "vlm");
}

/// Same contract for the asr binary (debug request-URL line and the
/// request-failure error lines).
#[test]
fn asr_failure_logs_do_not_leak_url_credentials() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let fixture = fixture();
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-asr"))
        .args([
            "--url",
            &credentialed_url("/v1/audio/transcriptions"),
            "--api-key",
            "dummy",
            "--scenario",
            "e2e-url-redaction",
            "--num-requests",
            "1",
            "--concurrency",
            "1",
            "--input",
            &format!("{manifest}/test-data/asr/input.jsonl"),
            "--ground-truth",
            &format!("{manifest}/test-data/asr/truth.jsonl"),
            "--model",
            "dummy",
            "--response-format",
            "verbose-json",
            "--connect-timeout",
            "2",
            "--data-log",
            fixture.data_log.to_str().unwrap(),
            "--debug-log",
            fixture.debug_log.to_str().unwrap(),
            "--error-log",
            fixture.error_log.to_str().unwrap(),
            "--log-level",
            "debug",
        ])
        .output()
        .expect("run asr");
    assert_no_secrets(&fixture, &output, "asr");
    assert_redacted_marker_present(&fixture, "asr");
}

/// Same contract for the imagegen binary: the reqwest error text written
/// into `error.log` for a failed generation must carry the redacted URL.
#[test]
fn imagegen_failure_logs_do_not_leak_url_credentials() {
    let fixture = fixture();
    let artifact_dir = fixture._dir.path().join("artifacts");
    let summary_json = fixture._dir.path().join("summary.json");
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-imagegen"))
        .args([
            "--url",
            &credentialed_url("/v1/images/generations"),
            "--api-key",
            "dummy",
            "--scenario",
            "e2e-url-redaction",
            "--model",
            "dummy",
            "--num-requests",
            "1",
            "--concurrency",
            "1",
            "--prompt",
            "a small test image",
            "--size",
            "64x64",
            "--connect-timeout",
            "2",
            "--data-log",
            fixture.data_log.to_str().unwrap(),
            "--summary-json",
            summary_json.to_str().unwrap(),
            "--artifact-dir",
            artifact_dir.to_str().unwrap(),
            "--error-log",
            fixture.error_log.to_str().unwrap(),
            "--debug-log",
            fixture.debug_log.to_str().unwrap(),
            "--no-save-images",
        ])
        .output()
        .expect("run imagegen");
    assert_no_secrets(&fixture, &output, "imagegen");
    assert_redacted_marker_present(&fixture, "imagegen");
}
