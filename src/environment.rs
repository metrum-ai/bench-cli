// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Reproducibility metadata captured with every shared summary.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Environment {
    pub hostname: Option<String>,
    pub os: &'static str,
    pub architecture: &'static str,
    pub cpu_cores: usize,
    pub rustc_version: String,
    pub package_version: &'static str,
    pub tokio_worker_threads: usize,
    pub tls_backend: &'static str,
    pub ntp_offset_ms: Option<i64>,
    pub server_model: Option<String>,
}

pub fn collect(ntp_offset_ms: Option<i64>, server_model: Option<String>) -> serde_json::Value {
    serde_json::to_value(Environment {
        hostname: hostname(),
        os: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
        cpu_cores: std::thread::available_parallelism()
            .map(std::num::NonZeroUsize::get)
            .unwrap_or(1),
        rustc_version: compile_time::rustc_version_str!().to_string(),
        package_version: env!("CARGO_PKG_VERSION"),
        tokio_worker_threads: std::thread::available_parallelism()
            .map(std::num::NonZeroUsize::get)
            .unwrap_or(1),
        tls_backend: "rustls",
        ntp_offset_ms,
        server_model,
    })
    .expect("Environment serialization cannot fail")
}

fn hostname() -> Option<String> {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok().filter(|s| !s.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_has_reproducibility_fields() {
        let value = collect(Some(3), Some("dummy".into()));
        assert_eq!(value["ntp_offset_ms"], 3);
        assert_eq!(value["server_model"], "dummy");
        assert!(value["cpu_cores"].as_u64().is_some_and(|n| n > 0));
        assert_eq!(value["tls_backend"], "rustls");
    }
}
