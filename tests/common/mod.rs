// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Shared harness for dummy-model-server end-to-end tests.
//!
//! Every helper degrades to `None` when the Go toolchain is absent so the
//! suite stays runnable on machines with Rust only.

#![allow(dead_code)]

use serde_json::Value;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

pub fn dummy_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("dummy-model-server")
}

pub struct Dummy {
    child: Child,
    pub port: u16,
}

impl Dummy {
    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, path)
    }
}

impl Drop for Dummy {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn go_available() -> bool {
    Command::new("go")
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// CI sets `METRUM_BENCH_REQUIRE_DUMMY=1` so a missing or unstartable server
/// fails the suite instead of silently skipping every assertion.
fn dummy_required() -> bool {
    std::env::var("METRUM_BENCH_REQUIRE_DUMMY").is_ok_and(|v| v != "0" && !v.is_empty())
}

/// Start the dummy server on a free port with `extra_args` appended.
/// Returns `None` when `go` is unavailable, so callers can skip.
pub fn spawn_dummy(extra_args: &[&str]) -> Option<Dummy> {
    if !go_available() {
        assert!(
            !dummy_required(),
            "METRUM_BENCH_REQUIRE_DUMMY is set but the go toolchain is missing"
        );
        return None;
    }
    let port = free_port();
    let mut args: Vec<String> = vec![
        "run".into(),
        "./cmd/dummy-model-server".into(),
        "-port".into(),
        port.to_string(),
    ];
    args.extend(extra_args.iter().map(|a| a.to_string()));

    let spawned = Command::new("go")
        .current_dir(dummy_dir())
        .args(&args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let child = match spawned {
        Ok(child) => child,
        Err(e) => {
            assert!(!dummy_required(), "could not start dummy-model-server: {e}");
            return None;
        }
    };
    let mut dummy = Dummy { child, port };

    let url = format!("http://127.0.0.1:{port}/v1/models");
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
        if reqwest::blocking::get(&url).is_ok_and(|resp| resp.status().is_success()) {
            return Some(dummy);
        }
        thread::sleep(Duration::from_millis(100));
    }
    let _ = dummy.child.kill();
    assert!(
        !dummy_required(),
        "dummy-model-server did not become ready on port {port} within 60s"
    );
    None
}

/// Per-request `request.v*` records from a `--data-log`, in file order.
pub fn request_records(data_log: &std::path::Path) -> Vec<Value> {
    let text = std::fs::read_to_string(data_log).expect("read data log");
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|v| {
            v.get("schema_version")
                .and_then(Value::as_str)
                .is_some_and(|s| s.contains("request.v"))
        })
        .collect()
}

/// The `summary.v*` record from a `--data-log`, if the run wrote one.
pub fn summary_record(data_log: &std::path::Path) -> Option<Value> {
    let text = std::fs::read_to_string(data_log).expect("read data log");
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|v| {
            v.get("schema_version")
                .and_then(Value::as_str)
                .is_some_and(|s| s.contains("summary.v"))
        })
}

/// Effective run configuration from `summary.v3.config`, with modality fields
/// flattened to the top level for test convenience.
pub fn run_config(data_log: &std::path::Path) -> Value {
    let summary = summary_record(data_log).expect("summary.v3 with config");
    let mut config = summary
        .get("config")
        .cloned()
        .expect("summary.config missing");
    if let Some(obj) = config.as_object_mut() {
        if let Some(modality) = obj.remove("modality") {
            if let Some(map) = modality.as_object() {
                for (k, v) in map {
                    obj.insert(k.clone(), v.clone());
                }
            }
        }
    }
    config
}

/// A 2x2 PNG, used where a test needs real image bytes on disk.
pub fn tiny_png() -> Vec<u8> {
    use std::io::Cursor;
    let image = image::RgbaImage::from_fn(2, 2, |x, y| {
        image::Rgba([(x * 120) as u8, (y * 120) as u8, 40, 255])
    });
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .expect("encode png");
    bytes.into_inner()
}

pub fn skip(reason: &str) {
    eprintln!("skipping: {reason}");
}
