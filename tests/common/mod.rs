// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Shared harness for dummy-model-server end-to-end tests.
//!
//! Every helper degrades to `None` when the Go toolchain is absent so the
//! suite stays runnable on machines with Rust only.

#![allow(dead_code)]

use serde_json::Value;
use std::io::{BufRead, BufReader, Read};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// Wait up to `timeout` for a line on `stream` containing `marker` and return
/// the port of the address that follows it (`127.0.0.1:41234` or `:41234`).
///
/// Servers are started on port 0 and announce the port they bound, so no test
/// reserves a port and drops it before the server binds (#211). A background
/// thread keeps draining the pipe afterwards so later writes never hit a
/// closed pipe. Returns `None` on timeout or if the stream closes first.
fn reported_port<R: Read + Send + 'static>(
    stream: R,
    marker: &'static str,
    timeout: Duration,
) -> Option<u16> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut tx = Some(tx);
        for line in BufReader::new(stream).lines() {
            let Ok(line) = line else { break };
            let Some(sender) = tx.as_ref() else { continue };
            let port = line.split_once(marker).and_then(|(_, rest)| {
                rest.split_whitespace()
                    .next()
                    .and_then(|addr| addr.rsplit(':').next())
                    .and_then(|port| port.parse::<u16>().ok())
            });
            if let Some(port) = port {
                let _ = sender.send(port);
                tx = None;
            }
        }
    });
    rx.recv_timeout(timeout).ok()
}

/// A running `metrum-ai-bench-cli-mock-server`, killed on drop.
pub struct Mock {
    child: Child,
    pub address: SocketAddr,
}

impl Drop for Mock {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Start the Rust mock server on `127.0.0.1:0` with `extra_args` appended and
/// return once it reports the port it bound.
pub fn spawn_mock(extra_args: &[&str]) -> Mock {
    let mut child = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-mock-server"))
        .args(["--listen", "127.0.0.1:0"])
        .args(extra_args)
        .stdout(Stdio::piped())
        .spawn()
        .expect("start mock server");
    let stdout = child.stdout.take().expect("mock stdout");
    let port = reported_port(stdout, "listening on ", Duration::from_secs(10));
    // Wrap first so the child is killed if the assertion below panics.
    let mut mock = Mock {
        child,
        address: SocketAddr::from(([127, 0, 0, 1], 0)),
    };
    mock.address
        .set_port(port.expect("mock server did not report its listen address"));
    mock
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
        kill_go_run(&mut self.child);
    }
}

/// Kill a `go run` child and the server binary it started. Killing only
/// `go run` leaves the compiled server orphaned, so on Unix its children are
/// killed first. Without `pkill` this degrades to killing `go run` alone.
fn kill_go_run(child: &mut Child) {
    #[cfg(unix)]
    let _ = Command::new("pkill")
        .args(["-KILL", "-P", &child.id().to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
    let _ = child.wait();
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

/// Start the dummy server on a free port in `-strict-media` mode with
/// `extra_args` appended, so tests send media a real server would accept.
/// Returns `None` when `go` is unavailable, so callers can skip.
pub fn spawn_dummy(extra_args: &[&str]) -> Option<Dummy> {
    let mut args = vec!["-strict-media"];
    args.extend_from_slice(extra_args);
    spawn_dummy_permissive(&args)
}

/// Start the dummy server without `-strict-media`. Only for tests that
/// deliberately send media a real server would reject.
pub fn spawn_dummy_permissive(extra_args: &[&str]) -> Option<Dummy> {
    if !go_available() {
        assert!(
            !dummy_required(),
            "METRUM_BENCH_REQUIRE_DUMMY is set but the go toolchain is missing"
        );
        return None;
    }
    let mut args: Vec<String> = vec![
        "run".into(),
        "./cmd/dummy-model-server".into(),
        "-port".into(),
        "0".into(),
    ];
    args.extend(extra_args.iter().map(|a| a.to_string()));

    let spawned = Command::new("go")
        .current_dir(dummy_dir())
        .args(&args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(e) => {
            assert!(!dummy_required(), "could not start dummy-model-server: {e}");
            return None;
        }
    };
    // The startup log line (stderr) carries the port the server bound. The
    // 60s budget covers `go run` compiling the server on a cold cache.
    let start = Instant::now();
    let budget = Duration::from_secs(60);
    let stderr = child.stderr.take().expect("dummy stderr");
    let Some(port) = reported_port(stderr, "dummy-model-server listening on ", budget) else {
        kill_go_run(&mut child);
        assert!(
            !dummy_required(),
            "dummy-model-server did not report its listen port within 60s"
        );
        return None;
    };
    let mut dummy = Dummy { child, port };

    let url = format!("http://127.0.0.1:{port}/v1/models");
    while start.elapsed() < budget {
        if reqwest::blocking::get(&url).is_ok_and(|resp| resp.status().is_success()) {
            return Some(dummy);
        }
        thread::sleep(Duration::from_millis(100));
    }
    kill_go_run(&mut dummy.child);
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

/// A 16 kHz mono 16-bit PCM WAV holding a 440 Hz sine tone, so ASR tests
/// upload audio that `-strict-media` (and a real server) can parse.
pub fn sine_wav(seconds: f64) -> Vec<u8> {
    const RATE: u32 = 16_000;
    let samples = (seconds * f64::from(RATE)) as u32;
    let data_len = samples * 2;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..samples {
        let t = f64::from(i) / f64::from(RATE);
        let v = (t * 440.0 * std::f64::consts::TAU).sin() * 0.3 * f64::from(i16::MAX);
        out.extend_from_slice(&(v as i16).to_le_bytes());
    }
    out
}

pub fn skip(reason: &str) {
    eprintln!("skipping: {reason}");
}
