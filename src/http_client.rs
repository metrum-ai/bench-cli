// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Shared reqwest client construction for the modality binaries.

use crate::connect_timing::{ConnectTimingLayer, TimedResolver};
use reqwest::Client;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// Options for [`build_http_client`]. Timeouts and pool settings mirror the
/// historical per-binary builders; TLS flags are opt-in and stamped into config.
#[derive(Debug, Clone)]
pub struct HttpClientOptions<'a> {
    pub request_timeout: Option<Duration>,
    pub connect_timeout: Duration,
    pub pool_max_idle_per_host: usize,
    pub pool_idle_timeout: Duration,
    pub tcp_keepalive: Duration,
    pub ca_cert: Option<&'a Path>,
    pub insecure: bool,
}

/// Build a rustls-backed HTTP client with optional private CA and insecure TLS.
///
/// Installs [`ConnectTimingLayer`] and [`TimedResolver`] so a per-request
/// [`crate::connect_timing::ConnectSlot`] can observe connect and DNS time and
/// connection reuse (pool hits report `0.0`).
///
/// When `opts.insecure` is true, certificate validation is disabled. That sink
/// is intentional: it backs the operator `--insecure` flag (stamped into
/// config), not a default path. CodeQL `rust/disabled-certificate-check` is
/// expected here and dismissed as wont_fix.
pub fn build_http_client(opts: HttpClientOptions<'_>) -> anyhow::Result<Client> {
    let mut builder = Client::builder()
        .connect_timeout(opts.connect_timeout)
        .pool_max_idle_per_host(opts.pool_max_idle_per_host)
        .pool_idle_timeout(opts.pool_idle_timeout)
        .tcp_keepalive(opts.tcp_keepalive)
        .dns_resolver(Arc::new(TimedResolver))
        .connector_layer(ConnectTimingLayer);

    if let Some(timeout) = opts.request_timeout {
        builder = builder.timeout(timeout);
    }

    // Intentional: only when opts.insecure (--insecure). See module docs above.
    if opts.insecure {
        builder = builder.danger_accept_invalid_certs(true);
    }

    if let Some(path) = opts.ca_cert {
        let pem = std::fs::read(path)
            .map_err(|e| anyhow::anyhow!("failed to read --ca-cert {}: {e}", path.display()))?;
        let cert = reqwest::Certificate::from_pem(&pem)
            .map_err(|e| anyhow::anyhow!("invalid --ca-cert {}: {e}", path.display()))?;
        builder = builder.add_root_certificate(cert);
    }

    builder
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build HTTP client: {e}"))
}
