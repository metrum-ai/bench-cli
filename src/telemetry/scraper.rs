// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Per-source Prometheus scrape tasks with startup probe and failure policy.

use super::config::{CompiledSource, TelemetryConfig, DEFAULT_REQUIRE_FAILURES};
use super::epoch::RunEpoch;
use super::parser::parse_exposition;
use super::row::{MetricType, Row, ScrapeErrorRow, TelemetryRow, TelemetrySourceStamp};
use super::writer::NdjsonWriter;
use crate::http_client::{build_http_client, HttpClientOptions};
use anyhow::{bail, Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, DATE};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::time::{interval, MissedTickBehavior};

#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub stamp: TelemetrySourceStamp,
}

/// Build a shared reqwest client for telemetry scrapes.
///
/// `insecure_tls` is honored via the shared [`build_http_client`] path (the
/// only approved `danger_accept_invalid_certs` call site). Gzip is enabled
/// only when TLS verification stays on, which matches the common local
/// exporter case (`http://127.0.0.1/...`).
pub fn build_telemetry_client(insecure_tls: bool, accept_gzip: bool) -> Result<reqwest::Client> {
    if insecure_tls {
        if accept_gzip {
            eprintln!(
                "telemetry: insecure_tls requested; using shared TLS client without gzip decode"
            );
        }
        return build_http_client(HttpClientOptions {
            request_timeout: Some(Duration::from_secs(30)),
            connect_timeout: Duration::from_secs(10),
            pool_max_idle_per_host: 8,
            pool_idle_timeout: Duration::from_secs(30),
            tcp_keepalive: Duration::from_secs(30),
            ca_cert: None,
            insecure: true,
        });
    }
    let mut builder = reqwest::Client::builder()
        .pool_max_idle_per_host(8)
        .tcp_keepalive(Duration::from_secs(30))
        .timeout(Duration::from_secs(30));
    if accept_gzip {
        builder = builder.gzip(true);
    }
    builder.build().context("build telemetry HTTP client")
}

fn auth_headers(src: &CompiledSource) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    if let Some(env_name) = &src.bearer_env {
        let token = std::env::var(env_name).with_context(|| {
            format!(
                "bearer_env {env_name} for telemetry source {} is unset",
                src.name
            )
        })?;
        let value = HeaderValue::from_str(&format!("Bearer {token}"))
            .context("invalid bearer token header")?;
        headers.insert(AUTHORIZATION, value);
    }
    if let Some(env_name) = &src.basic_auth_env {
        let raw = std::env::var(env_name).with_context(|| {
            format!(
                "basic_auth_env {env_name} for telemetry source {} is unset",
                src.name
            )
        })?;
        let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, raw);
        let value = HeaderValue::from_str(&format!("Basic {encoded}"))
            .context("invalid basic auth header")?;
        headers.insert(AUTHORIZATION, value);
    }
    Ok(headers)
}

fn clock_offset_ms(date_header: Option<&HeaderValue>) -> Option<i64> {
    let raw = date_header?.to_str().ok()?;
    let remote = httpdate_to_unix_ms(raw)?;
    let local = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis() as i64;
    Some(remote - local)
}

fn httpdate_to_unix_ms(raw: &str) -> Option<i64> {
    // Prefer chrono's RFC 2822 parser for HTTP Date.
    chrono::DateTime::parse_from_rfc2822(raw)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

async fn fetch_body(
    client: &reqwest::Client,
    src: &CompiledSource,
    timeout: Duration,
    max_body_bytes: u64,
) -> Result<(String, Option<i64>, u16)> {
    let headers = auth_headers(src)?;
    let response = client
        .get(&src.url)
        .headers(headers)
        .timeout(timeout)
        .send()
        .await
        .with_context(|| format!("GET {} ({})", src.url, src.name))?;
    let status = response.status().as_u16();
    let offset = clock_offset_ms(response.headers().get(DATE));
    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("HTTP {status} from {}: {}", src.name, truncate(&body, 200));
    }
    let bytes = response
        .bytes()
        .await
        .with_context(|| format!("read body from {}", src.name))?;
    if bytes.len() as u64 > max_body_bytes {
        bail!(
            "telemetry source {} response {} bytes exceeds max_body_bytes {max_body_bytes}",
            src.name,
            bytes.len()
        );
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Ok((text, offset, status))
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

/// Probe every source once. Fail fast on 4xx/5xx/connection refused.
/// Zero matches is an error unless `allow_empty`.
pub async fn probe_sources(
    client: &reqwest::Client,
    cfg: &TelemetryConfig,
    sources: &[CompiledSource],
) -> Result<Vec<ProbeResult>> {
    let timeout = Duration::from_millis(cfg.timeout_ms.min(cfg.default_interval_ms).max(1));
    let mut out = Vec::with_capacity(sources.len());
    for src in sources {
        let started = Instant::now();
        let (text, offset, _status) = fetch_body(client, src, timeout, cfg.max_body_bytes).await?;
        let parsed = parse_exposition(&text, &src.include);
        let matched = parsed.samples.len() as u64;
        eprintln!(
            "telemetry probe {}: matched_series={matched} scrape_ms={:.1} clock_offset_ms={:?}",
            src.name,
            started.elapsed().as_secs_f64() * 1000.0,
            offset
        );
        if matched == 0 && !src.allow_empty {
            bail!(
                "telemetry source {} matched zero series; set allow_empty: true to override",
                src.name
            );
        }
        out.push(ProbeResult {
            stamp: TelemetrySourceStamp {
                name: src.name.clone(),
                url: src.url.clone(),
                interval_ms: src.interval_ms,
                clock_offset_ms: offset,
                matched_series: Some(matched),
            },
        });
    }
    Ok(out)
}

struct SourceRuntime {
    consecutive_failures: AtomicU32,
}

/// Spawn one scrape task per source. Returns join handles.
#[allow(clippy::too_many_arguments)]
pub fn spawn_scrapers(
    client: reqwest::Client,
    cfg: TelemetryConfig,
    sources: Vec<CompiledSource>,
    epoch: Arc<RunEpoch>,
    run_id: Arc<String>,
    writer: NdjsonWriter,
    stop: Arc<AtomicBool>,
    require_telemetry: bool,
    require_failures: u32,
) -> Vec<tokio::task::JoinHandle<Result<()>>> {
    let require_failures = require_failures.max(1);
    sources
        .into_iter()
        .map(|src| {
            let client = client.clone();
            let cfg = cfg.clone();
            let epoch = Arc::clone(&epoch);
            let run_id = Arc::clone(&run_id);
            let writer = writer.clone();
            let stop = Arc::clone(&stop);
            tokio::spawn(async move {
                scrape_loop(
                    client,
                    cfg,
                    src,
                    epoch,
                    run_id,
                    writer,
                    stop,
                    require_telemetry,
                    require_failures,
                )
                .await
            })
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
async fn scrape_loop(
    client: reqwest::Client,
    cfg: TelemetryConfig,
    src: CompiledSource,
    epoch: Arc<RunEpoch>,
    run_id: Arc<String>,
    writer: NdjsonWriter,
    stop: Arc<AtomicBool>,
    require_telemetry: bool,
    require_failures: u32,
) -> Result<()> {
    let runtime = SourceRuntime {
        consecutive_failures: AtomicU32::new(0),
    };
    let timeout = Duration::from_millis(cfg.timeout_ms.min(src.interval_ms).max(1));
    let mut ticker = interval(Duration::from_millis(src.interval_ms));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    // Do not skip the first tick: short stages can finish before one interval.

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let started = Instant::now();
                let t_ns = epoch.elapsed_ns();
                match fetch_body(&client, &src, timeout, cfg.max_body_bytes).await {
                    Ok((text, _offset, _status)) => {
                        let scrape_ms = started.elapsed().as_secs_f64() * 1000.0;
                        let parsed = parse_exposition(&text, &src.include);
                        runtime.consecutive_failures.store(0, Ordering::Relaxed);
                        for sample in parsed.samples {
                            let (value, unit, raw) = apply_units(&src, &sample.metric, sample.value);
                            let _ = writer.try_send_telemetry(Row::Telemetry(TelemetryRow {
                                run_id: (*run_id).clone(),
                                t_ns,
                                src: src.name.clone(),
                                metric: sample.metric,
                                labels: sample.labels,
                                value,
                                unit,
                                mtype: sample.mtype,
                                scrape_ms,
                                raw,
                            }));
                        }
                    }
                    Err(err) => {
                        let failures = runtime.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
                        let http_status = extract_http_status(&err);
                        let _ = writer
                            .send_priority(Row::ScrapeError(ScrapeErrorRow {
                                run_id: (*run_id).clone(),
                                t_ns,
                                src: src.name.clone(),
                                http_status,
                                error: err.to_string(),
                            }))
                            .await;
                        if require_telemetry && failures >= require_failures {
                            stop.store(true, Ordering::Relaxed);
                            bail!(
                                "telemetry source {} failed {failures} consecutive scrapes (require-telemetry)",
                                src.name
                            );
                        }
                    }
                }
            }
            _ = async {
                while !stop.load(Ordering::Relaxed) {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            } => {
                break;
            }
        }
    }
    Ok(())
}

fn apply_units(src: &CompiledSource, metric: &str, value: f64) -> (f64, String, Option<f64>) {
    if let Some(scale) = src.units.get(metric) {
        let scaled = value * scale.scale;
        let raw = if (scale.scale - 1.0).abs() > f64::EPSILON {
            Some(value)
        } else {
            None
        };
        (scaled, scale.unit.clone(), raw)
    } else {
        (value, guess_unit(metric), None)
    }
}

fn guess_unit(metric: &str) -> String {
    let lower = metric.to_ascii_lowercase();
    if lower.contains("watts") || lower.ends_with("_w") || lower.contains("power_usage") {
        "W".into()
    } else if lower.contains("celsius") || lower.contains("_temp") {
        "C".into()
    } else if lower.contains("joules") || lower.ends_with("_j") {
        "J".into()
    } else if lower.contains("bytes") {
        "B".into()
    } else {
        "1".into()
    }
}

fn extract_http_status(err: &anyhow::Error) -> Option<u16> {
    let msg = err.to_string();
    if let Some(rest) = msg.strip_prefix("HTTP ") {
        rest.split_whitespace().next().and_then(|s| s.parse().ok())
    } else {
        None
    }
}

/// Last-seen metric values for optional request sugar (name -> value).
pub type LastSeenMap = Arc<tokio::sync::RwLock<std::collections::BTreeMap<String, f64>>>;

pub fn new_last_seen() -> LastSeenMap {
    Arc::new(tokio::sync::RwLock::new(std::collections::BTreeMap::new()))
}

#[allow(dead_code)]
pub fn metric_type_label(m: MetricType) -> &'static str {
    match m {
        MetricType::Counter => "counter",
        MetricType::Gauge => "gauge",
        MetricType::HistogramBucket => "histogram_bucket",
        MetricType::Summary => "summary",
        MetricType::Unknown => "unknown",
    }
}

pub fn default_require_failures() -> u32 {
    DEFAULT_REQUIRE_FAILURES
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::config::{
        TelemetryConfig, TelemetrySource, UnitScale, DEFAULT_MAX_BODY_BYTES,
        DEFAULT_REQUIRE_FAILURES,
    };
    use axum::body::Body;
    use axum::http::{header, StatusCode};
    use axum::response::Response;
    use axum::routing::get;
    use axum::Router;
    use regex::RegexSet;
    use std::collections::BTreeMap;
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::sync::Arc;
    use tempfile::NamedTempFile;
    use tokio::net::TcpListener;

    const FIXTURE: &str = r#"
# HELP DCGM_FI_DEV_POWER_USAGE Power draw
# TYPE DCGM_FI_DEV_POWER_USAGE gauge
DCGM_FI_DEV_POWER_USAGE{gpu="0"} 250.5
# HELP DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION Energy
# TYPE DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION counter
DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION{gpu="0"} 1000000
"#;

    async fn serve_metrics(
        status: StatusCode,
        body: &'static str,
        hits: Option<Arc<AtomicUsize>>,
    ) -> SocketAddr {
        let app = Router::new().route(
            "/metrics",
            get(move || {
                let hits = hits.clone();
                async move {
                    if let Some(h) = hits {
                        h.fetch_add(1, AtomicOrdering::Relaxed);
                    }
                    Response::builder()
                        .status(status)
                        .header(header::CONTENT_TYPE, "text/plain; version=0.0.4")
                        .header(header::DATE, "Wed, 24 Sep 2026 14:00:00 GMT")
                        .body(Body::from(body))
                        .unwrap()
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        addr
    }

    fn compiled(url: &str, allow_empty: bool) -> CompiledSource {
        let cfg = TelemetryConfig {
            default_interval_ms: 200,
            timeout_ms: 500,
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
            sources: vec![TelemetrySource {
                name: "dcgm".into(),
                url: url.into(),
                interval_ms: Some(100),
                include: vec!["^DCGM_FI_DEV_(POWER_USAGE|TOTAL_ENERGY_CONSUMPTION)$".into()],
                units: BTreeMap::from([(
                    "DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION".into(),
                    UnitScale {
                        scale: 0.001,
                        unit: "J".into(),
                    },
                )]),
                bearer_env: None,
                basic_auth_env: None,
                insecure_tls: false,
                allow_empty,
            }],
        };
        cfg.compile().expect("compile")[0].clone()
    }

    #[tokio::test]
    async fn probe_and_scrape_happy_path_writes_telemetry() {
        let addr = serve_metrics(StatusCode::OK, FIXTURE, None).await;
        let url = format!("http://{addr}/metrics");
        let client = build_telemetry_client(false, true).expect("client");
        let src = compiled(&url, false);
        let cfg = TelemetryConfig {
            default_interval_ms: 200,
            timeout_ms: 500,
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
            sources: vec![],
        };
        let probes = probe_sources(&client, &cfg, std::slice::from_ref(&src))
            .await
            .expect("probe");
        assert_eq!(probes.len(), 1);
        assert!(probes[0].stamp.matched_series.unwrap_or(0) >= 2);

        let file = NamedTempFile::new().expect("tmp");
        let (writer, handle) = NdjsonWriter::spawn(file.path().to_path_buf()).expect("spawn");
        let stop = Arc::new(AtomicBool::new(false));
        let epoch = Arc::new(RunEpoch::new());
        let run_id = Arc::new("run-probe".to_string());
        let handles = spawn_scrapers(
            client,
            cfg,
            vec![src],
            epoch,
            run_id,
            writer.clone(),
            Arc::clone(&stop),
            false,
            3,
        );
        tokio::time::sleep(Duration::from_millis(350)).await;
        stop.store(true, Ordering::Relaxed);
        for h in handles {
            let _ = h.await;
        }
        drop(writer);
        let stats = handle.shutdown().await.expect("shutdown");
        assert!(stats.written_rows >= 1, "expected telemetry rows");
        let text = std::fs::read_to_string(file.path()).expect("read");
        assert!(text.contains("\"kind\":\"telemetry\""));
        assert!(text.contains("DCGM_FI_DEV_POWER_USAGE"));
    }

    #[tokio::test]
    async fn probe_rejects_zero_matches_unless_allow_empty() {
        let addr = serve_metrics(StatusCode::OK, "# empty\n", None).await;
        let url = format!("http://{addr}/metrics");
        let client = build_telemetry_client(false, false).expect("client");
        let src = compiled(&url, false);
        let cfg = TelemetryConfig {
            default_interval_ms: 200,
            timeout_ms: 500,
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
            sources: vec![],
        };
        let err = probe_sources(&client, &cfg, &[src])
            .await
            .expect_err("zero matches");
        assert!(err.to_string().contains("matched zero series"));

        let src_ok = compiled(&url, true);
        probe_sources(&client, &cfg, &[src_ok])
            .await
            .expect("allow_empty");
    }

    #[tokio::test]
    async fn scrape_http_error_emits_scrape_error_and_can_abort() {
        let hits = Arc::new(AtomicUsize::new(0));
        let addr = serve_metrics(StatusCode::INTERNAL_SERVER_ERROR, "boom", Some(hits)).await;
        let url = format!("http://{addr}/metrics");
        let client = build_telemetry_client(false, false).expect("client");
        let src = compiled(&url, true);
        let cfg = TelemetryConfig {
            default_interval_ms: 100,
            timeout_ms: 200,
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
            sources: vec![],
        };
        let file = NamedTempFile::new().expect("tmp");
        let (writer, handle) = NdjsonWriter::spawn(file.path().to_path_buf()).expect("spawn");
        let stop = Arc::new(AtomicBool::new(false));
        let handles = spawn_scrapers(
            client,
            cfg,
            vec![src],
            Arc::new(RunEpoch::new()),
            Arc::new("run-fail".into()),
            writer.clone(),
            Arc::clone(&stop),
            true,
            2,
        );
        let join = handles.into_iter().next().unwrap();
        let result = tokio::time::timeout(Duration::from_secs(3), join)
            .await
            .expect("join timeout")
            .expect("task");
        assert!(result.is_err(), "require-telemetry should abort");
        assert!(stop.load(Ordering::Relaxed));
        drop(writer);
        let _ = handle.shutdown().await;
        let text = std::fs::read_to_string(file.path()).expect("read");
        assert!(text.contains("\"kind\":\"scrape_error\""));
        assert!(text.contains("HTTP 500") || text.contains("\"http_status\":500"));
    }

    #[tokio::test]
    async fn max_body_bytes_is_enforced() {
        let addr = serve_metrics(StatusCode::OK, FIXTURE, None).await;
        let url = format!("http://{addr}/metrics");
        let client = build_telemetry_client(false, false).expect("client");
        let mut src = compiled(&url, false);
        let cfg = TelemetryConfig {
            default_interval_ms: 200,
            timeout_ms: 500,
            max_body_bytes: 8,
            sources: vec![],
        };
        let err = fetch_body(&client, &src, Duration::from_secs(1), cfg.max_body_bytes)
            .await
            .expect_err("oversized");
        assert!(err.to_string().contains("max_body_bytes"));
        src.allow_empty = true;
        let _ = src;
    }

    #[test]
    fn unit_scaling_and_guesses() {
        let mut units = BTreeMap::new();
        units.insert(
            "DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION".into(),
            UnitScale {
                scale: 0.001,
                unit: "J".into(),
            },
        );
        let src = CompiledSource {
            name: "dcgm".into(),
            url: "http://127.0.0.1/metrics".into(),
            interval_ms: 250,
            include: RegexSet::new(["^DCGM_"]).expect("re"),
            units,
            bearer_env: None,
            basic_auth_env: None,
            insecure_tls: false,
            allow_empty: false,
        };
        let (v, u, raw) = apply_units(&src, "DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION", 1000.0);
        assert!((v - 1.0).abs() < 1e-9);
        assert_eq!(u, "J");
        assert_eq!(raw, Some(1000.0));
        assert_eq!(guess_unit("DCGM_FI_DEV_POWER_USAGE"), "W");
        assert_eq!(guess_unit("node_rapl_package_joules_total"), "J");
        assert_eq!(guess_unit("gpu_temp_celsius"), "C");
        assert_eq!(guess_unit("node_memory_MemAvailable_bytes"), "B");
        assert_eq!(guess_unit("vllm:num_requests_running"), "1");
        assert_eq!(
            extract_http_status(&anyhow::anyhow!("HTTP 503 from x")),
            Some(503)
        );
        assert_eq!(
            extract_http_status(&anyhow::anyhow!("connection refused")),
            None
        );
        assert_eq!(metric_type_label(MetricType::Gauge), "gauge");
        assert_eq!(default_require_failures(), DEFAULT_REQUIRE_FAILURES);
        let _ = new_last_seen();
    }

    #[tokio::test]
    async fn bearer_env_missing_fails_fetch() {
        let addr = serve_metrics(StatusCode::OK, FIXTURE, None).await;
        let url = format!("http://{addr}/metrics");
        let mut src = compiled(&url, false);
        src.bearer_env = Some("METRUM_TEST_MISSING_BEARER_ENV_XYZ".into());
        let client = build_telemetry_client(false, false).expect("client");
        let err = fetch_body(
            &client,
            &src,
            Duration::from_secs(1),
            DEFAULT_MAX_BODY_BYTES,
        )
        .await
        .expect_err("missing bearer");
        assert!(err.to_string().contains("bearer_env"));
    }

    #[tokio::test]
    async fn auth_headers_attach_bearer_and_basic() {
        std::env::set_var("METRUM_TEST_BEARER_TOKEN", "tok123");
        std::env::set_var("METRUM_TEST_BASIC_AUTH", "user:pass");
        let mut src = compiled("http://127.0.0.1/metrics", true);
        src.bearer_env = Some("METRUM_TEST_BEARER_TOKEN".into());
        let headers = auth_headers(&src).expect("bearer");
        assert!(headers
            .get(AUTHORIZATION)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("Bearer "));
        src.bearer_env = None;
        src.basic_auth_env = Some("METRUM_TEST_BASIC_AUTH".into());
        let headers = auth_headers(&src).expect("basic");
        assert!(headers
            .get(AUTHORIZATION)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("Basic "));
        std::env::remove_var("METRUM_TEST_BEARER_TOKEN");
        std::env::remove_var("METRUM_TEST_BASIC_AUTH");
    }
}
