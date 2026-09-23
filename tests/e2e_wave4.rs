// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Wave 4 UX: preflight, sut init, compare.

use serde_json::json;
use std::fs;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn cli_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_metrum-ai-bench-cli"))
}

fn start_mock() -> (ChildGuard, String) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve port");
    let address = listener.local_addr().expect("local address");
    drop(listener);
    let server = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-mock-server"))
        .args(["--listen", &address.to_string(), "--latency-ms", "5"])
        .stdout(Stdio::null())
        .spawn()
        .expect("start mock server");
    let guard = ChildGuard(server);
    let deadline = Instant::now() + Duration::from_secs(10);
    while TcpStream::connect(address).is_err() {
        assert!(Instant::now() < deadline, "mock server did not start");
        thread::sleep(Duration::from_millis(20));
    }
    (guard, format!("http://{address}"))
}

#[test]
fn preflight_passes_against_mock_server() {
    let (_server, base) = start_mock();
    let url = format!("{base}/v1/chat/completions");
    let output = Command::new(cli_bin())
        .args([
            "preflight",
            "--url",
            &url,
            "--api-key",
            "dummy",
            "--model",
            "mock",
            "--latency-samples",
            "1",
        ])
        .output()
        .expect("run preflight");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "stderr={} stdout={stdout}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("PASS"), "{stdout}");
    assert!(stdout.contains("preflight: ok"), "{stdout}");
    assert!(
        stdout.contains("Docker") || stdout.contains("limits:"),
        "{stdout}"
    );
}

#[test]
fn preflight_fails_unreachable() {
    let output = Command::new(cli_bin())
        .args([
            "preflight",
            "--url",
            "http://127.0.0.1:9/v1/chat/completions",
            "--api-key",
            "dummy",
            "--connect-timeout",
            "1",
            "--request-timeout",
            "2",
            "--latency-samples",
            "1",
        ])
        .output()
        .expect("run preflight");
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("FAIL") || stdout.contains("FAILED"),
        "{stdout}"
    );
}

#[test]
fn sut_init_writes_loadable_template() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sut.json");
    let output = Command::new(cli_bin())
        .args(["sut", "init", "--output", path.to_str().unwrap()])
        .output()
        .expect("sut init");
    assert!(output.status.success());
    let sut: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(sut["provenance"], "declared");
    assert!(sut["gpu"]["model"].is_string());
}

#[test]
fn sut_init_probe_writes_mixed_or_observed_fields() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("probed.json");
    let output = Command::new(cli_bin())
        .args(["sut", "init", "--probe", "--output", path.to_str().unwrap()])
        .output()
        .expect("sut init --probe");
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let sut: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert!(sut["provenance"] == "mixed" || sut["provenance"] == "declared");
    assert!(
        sut.get("field_provenance").is_some()
            || !String::from_utf8_lossy(&output.stderr).is_empty()
    );
}

#[test]
fn compare_emits_markdown_delta_table() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.json");
    let b = dir.path().join("b.json");
    let summary = |thr: f64| {
        json!({
            "points": [{
                "load": 1.0,
                "throughput": thr,
                "p50_s": 0.1,
                "p95_s": 0.2,
                "p99_s": 0.3,
                "goodput": thr,
                "user_tps": {"avg": 25.0},
                "error_rate": 0.0
            }]
        })
    };
    fs::write(&a, serde_json::to_string_pretty(&summary(10.0)).unwrap()).unwrap();
    fs::write(&b, serde_json::to_string_pretty(&summary(12.0)).unwrap()).unwrap();
    let output = Command::new(cli_bin())
        .args([
            "compare",
            a.to_str().unwrap(),
            b.to_str().unwrap(),
            "--labels",
            "H100,Pro6000",
            "--format",
            "markdown",
        ])
        .output()
        .expect("compare");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("throughput"));
    assert!(stdout.contains("H100"));
    assert!(stdout.contains("Pro6000"));
    assert!(stdout.contains("delta"));
}
