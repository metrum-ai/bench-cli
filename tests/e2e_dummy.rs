// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! End-to-end checks against dummy-model-server (Go). Skipped if `go` is missing.

use serde_json::Value;
use std::io::Write;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn llm_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_metrum-ai-bench-llm"))
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

fn dummy_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("dummy-model-server")
}

struct Dummy(Child);

impl Drop for Dummy {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_dummy(port: u16) -> Option<Dummy> {
    let status = Command::new("go")
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    let child = Command::new("go")
        .current_dir(dummy_dir())
        .args([
            "run",
            "./cmd/dummy-model-server",
            "-port",
            &port.to_string(),
            "-latency",
            "100ms",
            "-chunk-interval",
            "20ms",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let url = format!("http://127.0.0.1:{port}/v1/models");
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(30) {
        if let Ok(resp) = reqwest::blocking::get(&url) {
            if resp.status().is_success() {
                return Some(Dummy(child));
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    let mut child = child;
    let _ = child.kill();
    None
}

fn run_llm_against(port: u16) {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let prompts = tmp.path().join("prompts.jsonl");
    {
        let mut f = std::fs::File::create(&prompts).unwrap();
        writeln!(f, r#"{{"prompt":"Hi"}}"#).unwrap();
    }
    let data_log = tmp.path().join("out.jsonl");
    let status = Command::new(llm_bin())
        .args([
            "--url",
            &format!("http://127.0.0.1:{port}/v1/chat/completions"),
            "--api-key",
            "dummy",
            "--scenario",
            "e2e",
            "--num-requests",
            "1",
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
            "20",
            "--data-log",
            data_log.to_str().unwrap(),
            "--debug-log",
            tmp.path().join("debug.log").to_str().unwrap(),
            "--error-log",
            tmp.path().join("error.log").to_str().unwrap(),
            "--log-level",
            "error",
        ])
        .status()
        .expect("run llm");
    assert!(status.success(), "llm bench failed");
    let text = std::fs::read_to_string(&data_log).expect("read log");
    let mut ttft = None;
    let mut latency = None;
    let mut itl: Vec<f64> = Vec::new();
    for line in text.lines() {
        let v: Value = serde_json::from_str(line).expect("jsonl");
        if v.get("schema_version")
            .and_then(|s| s.as_str())
            .is_some_and(|s| s.contains("request.v2"))
        {
            ttft = v.get("ttft_s").and_then(|x| x.as_f64());
            latency = v.get("latency_s").and_then(|x| x.as_f64());
            if let Some(arr) = v.get("itl_s").and_then(|x| x.as_array()) {
                itl = arr.iter().filter_map(|x| x.as_f64()).collect();
            }
        }
    }
    let ttft_ms = ttft.expect("ttft") * 1000.0;
    let lat_ms = latency.expect("latency") * 1000.0;
    // Dummy: latency=100ms + first chunk 20ms => TTFT ~120ms; 20 tokens * 20ms + 100ms => ~500ms.
    assert!((100.0..200.0).contains(&ttft_ms), "ttft_ms={ttft_ms}");
    assert!((420.0..650.0).contains(&lat_ms), "latency_ms={lat_ms}");
    if !itl.is_empty() {
        let mean = itl.iter().sum::<f64>() / itl.len() as f64;
        assert!(
            (0.010..0.040).contains(&mean),
            "mean ITL {mean}s (want ~0.020s)"
        );
    }
}

#[test]
fn llm_streaming_dummy_timing() {
    let port = free_port();
    let Some(_dummy) = spawn_dummy(port) else {
        eprintln!("skipping: go dummy-model-server not available");
        return;
    };
    run_llm_against(port);
}
