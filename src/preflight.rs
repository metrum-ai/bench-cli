// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Serving-endpoint preflight: cheap client-side probes before a long run.
//!
//! Remote HTTP cannot audit CUDA toolchains, compilers, or GPU SKUs on the
//! serving host. Prefer vendor Docker images (see Platforms docs) when
//! reachability fails for toolchain reasons.

use crate::chat_stream;
use crate::http_client::{build_http_client, HttpClientOptions};
use anyhow::{Context, Result};
use reqwest::Client;
use serde::Serialize;
use serde_json::json;
use std::time::{Duration, Instant};

/// Docs pointer for remediation when the endpoint is unhealthy or unreachable.
pub const PLATFORMS_DOCKER_DOCS: &str =
    "https://docs.metrum.ai/metrum-ai-bench-cli/latest/docs/platforms/#serving-stacks-docker-first";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Fail,
    Warn,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreflightCheck {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreflightReport {
    pub schema_version: &'static str,
    pub url: String,
    pub model: String,
    pub checks: Vec<PreflightCheck>,
    pub all_passed: bool,
    pub limits: &'static str,
}

/// Normalize a user `--url` into a chat/completions endpoint.
pub fn normalize_chat_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.contains("/chat/completions") || trimmed.contains("/completions") {
        return trimmed.to_string();
    }
    if trimmed.ends_with("/v1") {
        return format!("{trimmed}/chat/completions");
    }
    format!("{trimmed}/v1/chat/completions")
}

fn remediation_docker() -> String {
    format!(
        "Prefer vendor Docker for vLLM/SGLang rather than bare-metal pip; see {PLATFORMS_DOCKER_DOCS}"
    )
}

/// Run staged probes against an OpenAI-compatible chat endpoint.
pub async fn run_preflight(
    url: &str,
    api_key: &str,
    model: &str,
    connect_timeout: Duration,
    request_timeout: Duration,
    latency_samples: u32,
) -> Result<PreflightReport> {
    let chat_url = normalize_chat_url(url);
    let client = build_http_client(HttpClientOptions {
        request_timeout: Some(request_timeout),
        connect_timeout,
        pool_max_idle_per_host: 2,
        pool_idle_timeout: Duration::from_secs(30),
        tcp_keepalive: Duration::from_secs(30),
        ca_cert: None,
        insecure: false,
    })?;

    let mut checks = Vec::new();

    checks.push(check_reachability(&client, &chat_url, connect_timeout).await);
    let reachable = checks.last().is_some_and(|c| c.status == CheckStatus::Pass);

    if reachable {
        checks.push(check_chat_unary(&client, &chat_url, api_key, model, request_timeout).await);
        checks.push(
            check_streaming_first_token(&client, &chat_url, api_key, model, request_timeout).await,
        );
        checks.push(
            check_latency_sample(
                &client,
                &chat_url,
                api_key,
                model,
                request_timeout,
                latency_samples.max(1),
            )
            .await,
        );
    } else {
        checks.push(PreflightCheck {
            name: "chat_probe".into(),
            status: CheckStatus::Fail,
            detail: "skipped: endpoint not reachable".into(),
            remediation: Some(remediation_docker()),
        });
        checks.push(PreflightCheck {
            name: "streaming_first_token".into(),
            status: CheckStatus::Fail,
            detail: "skipped: endpoint not reachable".into(),
            remediation: Some(remediation_docker()),
        });
        checks.push(PreflightCheck {
            name: "latency_sample".into(),
            status: CheckStatus::Fail,
            detail: "skipped: endpoint not reachable".into(),
            remediation: Some(remediation_docker()),
        });
    }

    let all_passed = checks.iter().all(|c| c.status != CheckStatus::Fail);
    Ok(PreflightReport {
        schema_version: "metrum-ai-bench-cli.preflight.v1",
        url: chat_url,
        model: model.to_string(),
        checks,
        all_passed,
        limits: "Client HTTP probes only. Cannot observe CUDA toolkit, compiler, SM vs wheel, or remote GPU inventory. Prefer vendor Docker images on the serving host.",
    })
}

async fn check_reachability(
    client: &Client,
    url: &str,
    connect_timeout: Duration,
) -> PreflightCheck {
    let started = Instant::now();
    // GET on chat paths often returns 404/405; any TCP/TLS response proves reachability.
    let result = client.get(url).timeout(connect_timeout).send().await;
    match result {
        Ok(resp) => {
            let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
            let status = resp.status();
            if status.is_server_error() {
                PreflightCheck {
                    name: "reachability".into(),
                    status: CheckStatus::Fail,
                    detail: format!("HTTP {status} in {elapsed_ms:.0} ms (server error on probe)"),
                    remediation: Some(remediation_docker()),
                }
            } else {
                PreflightCheck {
                    name: "reachability".into(),
                    status: CheckStatus::Pass,
                    detail: format!("HTTP {status} in {elapsed_ms:.0} ms"),
                    remediation: None,
                }
            }
        }
        Err(err) => PreflightCheck {
            name: "reachability".into(),
            status: CheckStatus::Fail,
            detail: format!("connect/request failed: {err}"),
            remediation: Some(remediation_docker()),
        },
    }
}

fn chat_body(model: &str, stream: bool, max_tokens: u32) -> serde_json::Value {
    json!({
        "model": model,
        "stream": stream,
        "max_tokens": max_tokens,
        "temperature": 0.0,
        "messages": [{"role": "user", "content": "ping"}]
    })
}

async fn check_chat_unary(
    client: &Client,
    url: &str,
    api_key: &str,
    model: &str,
    request_timeout: Duration,
) -> PreflightCheck {
    let started = Instant::now();
    let body = chat_body(model, false, 4);
    match client
        .post(url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(request_timeout)
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                return PreflightCheck {
                    name: "chat_probe".into(),
                    status: CheckStatus::Fail,
                    detail: format!(
                        "HTTP {status} in {elapsed_ms:.0} ms: {}",
                        truncate(&text, 160)
                    ),
                    remediation: Some(remediation_docker()),
                };
            }
            let parsed: Result<serde_json::Value, _> = serde_json::from_str(&text);
            match parsed {
                Ok(value) if value.get("choices").is_some() => PreflightCheck {
                    name: "chat_probe".into(),
                    status: CheckStatus::Pass,
                    detail: format!("unary chat OK in {elapsed_ms:.0} ms"),
                    remediation: None,
                },
                Ok(_) => PreflightCheck {
                    name: "chat_probe".into(),
                    status: CheckStatus::Warn,
                    detail: format!(
                        "HTTP 2xx without choices in {elapsed_ms:.0} ms (non-OpenAI shape?)"
                    ),
                    remediation: Some(
                        "Confirm --url points at an OpenAI-compatible /v1/chat/completions path"
                            .into(),
                    ),
                },
                Err(err) => PreflightCheck {
                    name: "chat_probe".into(),
                    status: CheckStatus::Fail,
                    detail: format!("invalid JSON body ({err})"),
                    remediation: Some(remediation_docker()),
                },
            }
        }
        Err(err) => PreflightCheck {
            name: "chat_probe".into(),
            status: CheckStatus::Fail,
            detail: format!("request failed: {err}"),
            remediation: Some(remediation_docker()),
        },
    }
}

async fn check_streaming_first_token(
    client: &Client,
    url: &str,
    api_key: &str,
    model: &str,
    request_timeout: Duration,
) -> PreflightCheck {
    let started = Instant::now();
    let body = chat_body(model, true, 8);
    let response = match client
        .post(url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(request_timeout)
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(err) => {
            return PreflightCheck {
                name: "streaming_first_token".into(),
                status: CheckStatus::Fail,
                detail: format!("request failed: {err}"),
                remediation: Some(remediation_docker()),
            };
        }
    };
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return PreflightCheck {
            name: "streaming_first_token".into(),
            status: CheckStatus::Fail,
            detail: format!("HTTP {status}: {}", truncate(&text, 160)),
            remediation: Some(remediation_docker()),
        };
    }
    match chat_stream::consume(response.bytes_stream(), started).await {
        Ok(result) => {
            let ttft_ms = result.ttft.as_secs_f64() * 1000.0;
            PreflightCheck {
                name: "streaming_first_token".into(),
                status: CheckStatus::Pass,
                detail: format!("first visible token in {ttft_ms:.0} ms"),
                remediation: None,
            }
        }
        Err(err) => PreflightCheck {
            name: "streaming_first_token".into(),
            status: CheckStatus::Fail,
            detail: format!("stream smoke failed: {err}"),
            remediation: Some(format!(
                "Streaming may be broken or gateways may synthesize SSE; check engine flags. {}",
                remediation_docker()
            )),
        },
    }
}

async fn check_latency_sample(
    client: &Client,
    url: &str,
    api_key: &str,
    model: &str,
    request_timeout: Duration,
    samples: u32,
) -> PreflightCheck {
    let mut latencies_ms = Vec::with_capacity(samples as usize);
    for _ in 0..samples {
        let started = Instant::now();
        let body = chat_body(model, false, 4);
        match client
            .post(url)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", "application/json")
            .json(&body)
            .timeout(request_timeout)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                let _ = resp.bytes().await;
                latencies_ms.push(started.elapsed().as_secs_f64() * 1000.0);
            }
            Ok(resp) => {
                return PreflightCheck {
                    name: "latency_sample".into(),
                    status: CheckStatus::Fail,
                    detail: format!("sample HTTP {}", resp.status()),
                    remediation: Some(remediation_docker()),
                };
            }
            Err(err) => {
                return PreflightCheck {
                    name: "latency_sample".into(),
                    status: CheckStatus::Fail,
                    detail: format!("sample failed: {err}"),
                    remediation: Some(remediation_docker()),
                };
            }
        }
    }
    latencies_ms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = latencies_ms[latencies_ms.len() / 2];
    let status = if median > 30_000.0 {
        CheckStatus::Warn
    } else {
        CheckStatus::Pass
    };
    PreflightCheck {
        name: "latency_sample".into(),
        status,
        detail: format!(
            "n={samples} median={median:.0} ms (min={:.0} max={:.0})",
            latencies_ms.first().copied().unwrap_or(0.0),
            latencies_ms.last().copied().unwrap_or(0.0)
        ),
        remediation: if status == CheckStatus::Warn {
            Some(
                "Median unary latency over 30s; check cold load, WAN RTT, or serve path (Docker-first Platforms docs)"
                    .into(),
            )
        } else {
            None
        },
    }
}

fn truncate(text: &str, max: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if flat.len() <= max {
        flat
    } else {
        format!("{}...", &flat[..max])
    }
}

/// Render a pass/fail table for humans (not only JSON).
pub fn format_table(report: &PreflightReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "preflight: {}  model={}\n",
        report.url, report.model
    ));
    out.push_str(&format!("{:<24} {:<6} {}\n", "CHECK", "STATUS", "DETAIL"));
    out.push_str(&format!("{:-<24} {:-<6} {:-<40}\n", "", "", ""));
    for check in &report.checks {
        let status = match check.status {
            CheckStatus::Pass => "PASS",
            CheckStatus::Fail => "FAIL",
            CheckStatus::Warn => "WARN",
        };
        out.push_str(&format!(
            "{:<24} {:<6} {}\n",
            check.name, status, check.detail
        ));
        if let Some(rem) = &check.remediation {
            out.push_str(&format!("  remediation: {rem}\n"));
        }
    }
    out.push_str(&format!("\nlimits: {}\n", report.limits));
    if report.all_passed {
        out.push_str("preflight: ok\n");
    } else {
        out.push_str("preflight: FAILED\n");
    }
    out
}

/// Convenience wrapper used by the unified CLI.
pub fn run_preflight_blocking(
    url: &str,
    api_key: &str,
    model: &str,
    connect_timeout_s: u64,
    request_timeout_s: u64,
    latency_samples: u32,
) -> Result<PreflightReport> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to start tokio runtime for preflight")?;
    rt.block_on(run_preflight(
        url,
        api_key,
        model,
        Duration::from_secs(connect_timeout_s),
        Duration::from_secs(request_timeout_s),
        latency_samples,
    ))
}

/// Exit code helper: non-zero when any check failed.
pub fn exit_failure(report: &PreflightReport) -> bool {
    !report.all_passed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_base_and_v1_urls() {
        assert_eq!(
            normalize_chat_url("http://127.0.0.1:8000"),
            "http://127.0.0.1:8000/v1/chat/completions"
        );
        assert_eq!(
            normalize_chat_url("http://127.0.0.1:8000/v1"),
            "http://127.0.0.1:8000/v1/chat/completions"
        );
        assert_eq!(
            normalize_chat_url("http://127.0.0.1:8000/v1/chat/completions"),
            "http://127.0.0.1:8000/v1/chat/completions"
        );
    }

    #[test]
    fn table_marks_failure() {
        let report = PreflightReport {
            schema_version: "metrum-ai-bench-cli.preflight.v1",
            url: "http://x/v1/chat/completions".into(),
            model: "m".into(),
            checks: vec![PreflightCheck {
                name: "reachability".into(),
                status: CheckStatus::Fail,
                detail: "down".into(),
                remediation: Some(remediation_docker()),
            }],
            all_passed: false,
            limits: "limits",
        };
        let table = format_table(&report);
        assert!(table.contains("FAIL"));
        assert!(table.contains("preflight: FAILED"));
        assert!(table.contains("Docker"));
    }

    #[test]
    fn unreachable_host_fails_preflight() {
        let report = run_preflight_blocking(
            "http://127.0.0.1:9/v1/chat/completions",
            "dummy",
            "dummy",
            1,
            2,
            1,
        )
        .expect("preflight runs");
        assert!(!report.all_passed);
        assert_eq!(report.checks[0].name, "reachability");
        assert_eq!(report.checks[0].status, CheckStatus::Fail);
    }
}
