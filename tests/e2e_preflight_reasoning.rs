// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! #230 end-to-end checks for the preflight streaming probe against
//! dummy-model-server in `-strict-media` mode (Metrum AI Bench). A thinking
//! model can spend the probe's whole `max_tokens` on reasoning, and that still
//! proves streaming works. Skipped if `go` is missing.

mod common;

use common::{skip, spawn_dummy};
use serde_json::Value;
use std::process::Command;

/// Run `metrum-ai-bench-cli preflight --json` and return (exit ok, the
/// `streaming_first_token` check).
fn preflight_streaming(url: &str) -> (bool, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli"))
        .args([
            "preflight",
            "--url",
            url,
            "--api-key",
            "dummy",
            "--model",
            "dummy",
            "--latency-samples",
            "1",
            "--json",
        ])
        .output()
        .expect("run preflight");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // The human table comes first, and its remediation lines can hold JSON,
    // so the pretty-printed report starts at the first line that is `{`.
    let json_start = stdout.find("\n{\n").expect("json report") + 1;
    let report: Value = serde_json::from_str(&stdout[json_start..]).expect("report json");
    let check = report["checks"]
        .as_array()
        .expect("checks")
        .iter()
        .find(|c| c["name"] == "streaming_first_token")
        .cloned()
        .expect("streaming_first_token check");
    (output.status.success(), check)
}

#[test]
fn preflight_passes_on_reasoning_only_stream() {
    let Some(dummy) = spawn_dummy(&["-reasoning-only", "-chunk-interval", "2ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let (ok, check) = preflight_streaming(&dummy.url("/v1/chat/completions"));
    assert!(ok, "{check}");
    assert_eq!(check["status"], "pass", "{check}");
    assert!(
        check["detail"]
            .as_str()
            .is_some_and(|d| d.starts_with("first token was reasoning")),
        "{check}"
    );
}

#[test]
fn preflight_reports_visible_token_after_reasoning() {
    let Some(dummy) = spawn_dummy(&["-reasoning", "-chunk-interval", "2ms"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let (ok, check) = preflight_streaming(&dummy.url("/v1/chat/completions"));
    assert!(ok, "{check}");
    assert_eq!(check["status"], "pass", "{check}");
    assert!(
        check["detail"]
            .as_str()
            .is_some_and(|d| d.starts_with("first visible token")),
        "{check}"
    );
}

#[test]
fn preflight_fails_on_empty_stream() {
    let Some(dummy) = spawn_dummy(&["-role-only"]) else {
        skip("go dummy-model-server not available");
        return;
    };
    let (ok, check) = preflight_streaming(&dummy.url("/v1/chat/completions"));
    assert!(!ok, "{check}");
    assert_eq!(check["status"], "fail", "{check}");
    assert!(
        check["remediation"]
            .as_str()
            .is_some_and(|r| r.contains("enable_thinking")),
        "{check}"
    );
}
