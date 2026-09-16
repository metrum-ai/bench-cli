// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Strategic benchmark primitives: sweeps, validity, server correlation and exports.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchRecord {
    pub seq: u64,
    pub stage: f64,
    pub endpoint: String,
    pub scheduled_unix_ns: u128,
    pub sent_unix_ns: u128,
    /// Scheduled-to-completion latency. This includes client queueing delay.
    pub latency_s: f64,
    /// Scheduled-to-send delay, exposing coordinated omission under overload.
    pub queue_delay_s: f64,
    /// Send-to-completion service latency.
    pub service_latency_s: f64,
    pub success: bool,
    pub valid: Option<bool>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub session_id: Option<String>,
    pub turn: Option<usize>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SweepPoint {
    pub load: f64,
    pub n: usize,
    pub errors: usize,
    pub throughput: f64,
    /// Type-7 latency distribution over successful requests.
    pub latency_s: crate::stats::DistSummary,
    pub p50_s: Option<f64>,
    pub p95_s: Option<f64>,
    pub p99_s: Option<f64>,
    pub p99_unreliable: bool,
    pub error_rate: Option<f64>,
    pub validity_rate: Option<f64>,
    /// Schema-valid successes that also meet optional `--slo` thresholds.
    /// Without SLOs this equals validity-filtered throughput (often == throughput).
    pub goodput: f64,
    /// True when no SLO thresholds were applied (goodput is validity-only).
    pub goodput_equals_throughput: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slo_thresholds_s: Option<std::collections::BTreeMap<String, f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
}

fn strategic_meets_slos(record: &BenchRecord, slos: &crate::summary::SloConfig) -> bool {
    if let Some(limit) = slos.e2e_s {
        if record.latency_s > limit {
            return false;
        }
    }
    // Strategic records do not carry TTFT/TPOT; those SLO keys are ignored here.
    true
}

pub fn summarize_stage(
    load: f64,
    records: &[BenchRecord],
    seconds: f64,
    slos: &crate::summary::SloConfig,
    config: Option<Value>,
) -> SweepPoint {
    let success_lats: Vec<f64> = records
        .iter()
        .filter(|record| record.success)
        .map(|record| record.latency_s)
        .collect();
    let latency_s = crate::stats::DistSummary::from_values(&success_lats);
    let successes = success_lats.len();
    let errors = records.len().saturating_sub(successes);
    let valid = records
        .iter()
        .filter(|record| record.success && record.valid.unwrap_or(true))
        .count();
    let validity_count = records
        .iter()
        .filter(|record| record.valid.is_some())
        .count();
    let good = records
        .iter()
        .filter(|record| {
            record.success && record.valid.unwrap_or(true) && strategic_meets_slos(record, slos)
        })
        .count();
    let elapsed = seconds.max(f64::EPSILON);
    let thresholds: std::collections::BTreeMap<String, f64> = [
        ("ttft", slos.ttft_s),
        ("tpot", slos.tpot_s),
        ("e2e", slos.e2e_s),
    ]
    .into_iter()
    .filter_map(|(name, value)| value.map(|v| (name.to_string(), v)))
    .collect();
    let no_slos = thresholds.is_empty();
    SweepPoint {
        load,
        n: records.len(),
        errors,
        throughput: successes as f64 / elapsed,
        latency_s: latency_s.clone(),
        p50_s: latency_s.p50,
        p95_s: latency_s.p95,
        p99_s: latency_s.p99,
        p99_unreliable: latency_s.p99_unreliable,
        error_rate: if records.is_empty() {
            None
        } else {
            Some(errors as f64 / records.len() as f64)
        },
        validity_rate: (validity_count > 0).then_some(valid as f64 / successes.max(1) as f64),
        goodput: good as f64 / elapsed,
        goodput_equals_throughput: no_slos,
        slo_thresholds_s: (!no_slos).then_some(thresholds),
        config,
    }
}

/// Finds the maximum distance from the endpoint chord after normalizing the
/// throughput/latency curve. This is the standard deterministic Kneedle
/// construction and is robust to units and uneven sweep spacing.
pub fn detect_knee(points: &[SweepPoint]) -> Option<usize> {
    if points.len() < 3 {
        return None;
    }
    let x_min = points.first()?.throughput;
    let x_max = points.last()?.throughput;
    let y_min = points.first()?.p95_s?;
    let y_max = points.last()?.p95_s?;
    if (x_max - x_min).abs() <= f64::EPSILON || (y_max - y_min).abs() <= f64::EPSILON {
        return None;
    }
    points
        .iter()
        .enumerate()
        .skip(1)
        .take(points.len() - 2)
        .filter_map(|(index, point)| {
            let x = (point.throughput - x_min) / (x_max - x_min);
            point
                .p95_s
                .map(|latency| (index, ((latency - y_min) / (y_max - y_min) - x).abs()))
        })
        .max_by(|left, right| left.1.total_cmp(&right.1))
        .map(|pair| pair.0)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerMetrics {
    pub kv_cache_usage: Option<f64>,
    pub preemptions: Option<f64>,
    pub requests_running: Option<f64>,
    pub requests_waiting: Option<f64>,
}

fn prometheus_values<'a>(text: &'a str, names: &'a [&'a str]) -> impl Iterator<Item = f64> + 'a {
    text.lines().filter_map(|line| {
        let line = line.trim();
        let name = line.split(['{', ' ', '\t']).next().unwrap_or_default();
        (!line.starts_with('#') && names.contains(&name))
            .then(|| line.split_whitespace().last()?.parse().ok())
            .flatten()
    })
}

pub fn parse_server_metrics(text: &str) -> ServerMetrics {
    ServerMetrics {
        kv_cache_usage: prometheus_values(
            text,
            &[
                "vllm:gpu_cache_usage_perc",
                "vllm:kv_cache_usage_perc",
                "sglang:cache_hit_rate",
                "trtllm_kv_cache_utilization",
            ],
        )
        .max_by(f64::total_cmp),
        preemptions: prometheus_values(
            text,
            &[
                "vllm:num_preemptions_total",
                "sglang:num_preemptions_total",
                "trtllm_request_preemptions",
            ],
        )
        .reduce(|sum, value| sum + value),
        requests_running: prometheus_values(
            text,
            &[
                "vllm:num_requests_running",
                "sglang:num_running_reqs",
                "trtllm_request_metrics_active",
            ],
        )
        .reduce(|sum, value| sum + value),
        requests_waiting: prometheus_values(
            text,
            &[
                "vllm:num_requests_waiting",
                "sglang:num_queue_reqs",
                "trtllm_request_metrics_queued",
            ],
        )
        .reduce(|sum, value| sum + value),
    }
}

pub async fn scrape_metrics(client: &reqwest::Client, url: &str) -> Result<ServerMetrics> {
    let text = client
        .get(url)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .with_context(|| format!("scrape {url}"))?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_server_metrics(&text))
}

#[derive(Debug, Clone, Deserialize)]
pub struct Session {
    pub session_id: String,
    pub messages: Vec<Value>,
}

pub fn load_sessions(path: &Path) -> Result<Vec<Session>> {
    let reader = BufReader::new(File::open(path)?);
    let mut sessions = Vec::new();
    for (line_number, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let session: Session = serde_json::from_str(&line)
            .with_context(|| format!("invalid session JSONL line {}", line_number + 1))?;
        if session.messages.is_empty() {
            bail!("session {} has no messages", session.session_id);
        }
        sessions.push(session);
    }
    if sessions.is_empty() {
        bail!("session dataset is empty");
    }
    Ok(sessions)
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum PrefixControl {
    Shared,
    Unique,
    None,
}

pub fn controlled_messages(
    messages: &[Value],
    shared_prefix: Option<&str>,
    control: PrefixControl,
    session_id: &str,
) -> Vec<Value> {
    let mut output = Vec::new();
    if let Some(prefix) = shared_prefix {
        if !matches!(control, PrefixControl::None) {
            let content = match control {
                PrefixControl::Shared => prefix.to_string(),
                PrefixControl::Unique => format!("[session:{session_id}] {prefix}"),
                PrefixControl::None => unreachable!(),
            };
            output.push(json!({"role": "system", "content": content}));
        }
    }
    output.extend_from_slice(messages);
    output
}

#[derive(Debug, Clone)]
pub enum Validity {
    Json {
        validator: Arc<jsonschema::Validator>,
    },
    ToolCall {
        validators: BTreeMap<String, Arc<jsonschema::Validator>>,
    },
}

impl Validity {
    pub fn json_schema(schema: &Value) -> Result<Self> {
        let validator = jsonschema::validator_for(schema)
            .map_err(|error| anyhow::anyhow!("invalid JSON schema: {error}"))?;
        Ok(Self::Json {
            validator: Arc::new(validator),
        })
    }

    pub fn tool_names(tools: &Value) -> Result<Self> {
        let array = tools.as_array().context("tools must be a JSON array")?;
        if array.is_empty() {
            bail!("tools array must not be empty");
        }
        let mut validators = BTreeMap::new();
        for tool in array {
            let name = tool
                .pointer("/function/name")
                .and_then(Value::as_str)
                .context("each tool must have function.name")?;
            let parameters = tool
                .pointer("/function/parameters")
                .cloned()
                .unwrap_or_else(|| json!({"type":"object"}));
            let validator = jsonschema::validator_for(&parameters)
                .map_err(|error| anyhow::anyhow!("invalid schema for tool {name}: {error}"))?;
            if validators
                .insert(name.to_string(), Arc::new(validator))
                .is_some()
            {
                bail!("duplicate tool name: {name}");
            }
        }
        Ok(Self::ToolCall { validators })
    }

    pub fn validate(&self, response: &Value) -> bool {
        match self {
            Self::Json { validator } => {
                let content = response
                    .pointer("/choices/0/message/content")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                serde_json::from_str::<Value>(content)
                    .ok()
                    .is_some_and(|value| validator.is_valid(&value))
            }
            Self::ToolCall { validators } => response
                .pointer("/choices/0/message/tool_calls")
                .and_then(Value::as_array)
                .is_some_and(|calls| {
                    !calls.is_empty()
                        && calls.iter().all(|call| {
                            let name = call.pointer("/function/name").and_then(Value::as_str);
                            let arguments =
                                call.pointer("/function/arguments").and_then(Value::as_str);
                            name.and_then(|name| validators.get(name))
                                .zip(
                                    arguments
                                        .and_then(|text| serde_json::from_str::<Value>(text).ok()),
                                )
                                .is_some_and(|(validator, arguments)| {
                                    validator.is_valid(&arguments)
                                })
                        })
                }),
        }
    }
}

pub fn export_csv(path: &Path, records: &[BenchRecord]) -> Result<()> {
    let mut writer = csv::Writer::from_path(path)?;
    for record in records {
        writer.serialize(record)?;
    }
    writer.flush()?;
    Ok(())
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn export_html(
    path: &Path,
    title: &str,
    points: &[SweepPoint],
    knee: Option<usize>,
    server: &ServerMetrics,
) -> Result<()> {
    let width = 760.0;
    let height = 300.0;
    let max_x = points
        .iter()
        .map(|point| point.throughput)
        .fold(0.0_f64, f64::max)
        .max(1.0);
    let max_y = points
        .iter()
        .filter_map(|point| point.p95_s)
        .fold(0.0_f64, f64::max)
        .max(0.001);
    let coordinates: Vec<_> = points
        .iter()
        .map(|point| {
            (
                40.0 + point.throughput / max_x * (width - 60.0),
                height - 30.0 - point.p95_s.unwrap_or_default() / max_y * (height - 50.0),
            )
        })
        .collect();
    let polyline = coordinates
        .iter()
        .map(|(x, y)| format!("{x:.1},{y:.1}"))
        .collect::<Vec<_>>()
        .join(" ");
    let rows = points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            format!(
                "<tr{}><td>{:.2}</td><td>{}</td><td>{:.2}</td><td>{}</td><td>{}</td><td>{}</td><td>{:.2}</td></tr>",
                if knee == Some(index) {
                    " class=knee"
                } else {
                    ""
                },
                point.load,
                point.n,
                point.throughput,
                point
                    .p95_s
                    .map(|value| format!("{value:.3}"))
                    .unwrap_or_else(|| "—".to_string()),
                point
                    .p99_s
                    .map(|value| format!("{value:.3}"))
                    .unwrap_or_else(|| "—".to_string()),
                point
                    .error_rate
                    .map(|value| format!("{:.2}%", value * 100.0))
                    .unwrap_or_else(|| "—".to_string()),
                point.goodput
            )
        })
        .collect::<String>();
    let knee_circle = knee
        .and_then(|index| coordinates.get(index))
        .map(|(x, y)| format!(r##"<circle cx="{x:.1}" cy="{y:.1}" r="7" fill="#ef4444"/>"##))
        .unwrap_or_default();
    let html = format!(
        r##"<!doctype html><html><head><meta charset="utf-8"><title>{}</title>
<style>body{{font:14px system-ui;margin:2rem;max-width:900px}}table{{border-collapse:collapse;width:100%}}th,td{{padding:.45rem;border-bottom:1px solid #ddd;text-align:right}}th:first-child,td:first-child{{text-align:left}}.knee{{background:#fee2e2}}svg{{border:1px solid #ddd;background:#fafafa}}</style></head>
<body><h1>{}</h1><p>Latency-throughput curve; red marks the automatically detected knee.</p>
<svg viewBox="0 0 {width} {height}" role="img" aria-label="p95 latency by throughput"><polyline points="{polyline}" fill="none" stroke="#2563eb" stroke-width="3"/>{knee_circle}</svg>
<h2>Sweep</h2><table><thead><tr><th>Load</th><th>n</th><th>Throughput</th><th>p95 seconds</th><th>p99 seconds</th><th>Error</th><th>Goodput</th></tr></thead><tbody>{rows}</tbody></table>
<h2>Server correlation</h2><pre>{}</pre></body></html>"##,
        escape_html(title),
        escape_html(title),
        escape_html(&serde_json::to_string_pretty(server)?),
    );
    std::fs::write(path, html)?;
    Ok(())
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum MlperfScenario {
    Server,
    Offline,
}

pub fn export_mlperf(
    directory: &Path,
    scenario: MlperfScenario,
    records: &[BenchRecord],
    duration_s: f64,
) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    let completed = records.iter().filter(|record| record.success).count();
    let qps = completed as f64 / duration_s.max(f64::EPSILON);
    let latencies = crate::stats::sort_finite(
        records
            .iter()
            .filter(|record| record.success)
            .map(|record| record.latency_s),
    );
    let scenario_name = match scenario {
        MlperfScenario::Server => "Server",
        MlperfScenario::Offline => "Offline",
    };
    const DISCLAIMER: &str = "UNOFFICIAL: This is NOT an audited or submitted MLPerf result. Parser-oriented interoperability export only; do not treat as an official MLPerf LoadGen run.";
    let mut summary = File::create(directory.join("mlperf_log_summary.txt"))?;
    writeln!(summary, "{DISCLAIMER}")?;
    writeln!(summary, "MLPerf Results Summary")?;
    writeln!(summary, "SUT name : MetrumBench")?;
    writeln!(summary, "Scenario : {scenario_name}")?;
    writeln!(summary, "Mode : PerformanceOnly")?;
    match scenario {
        MlperfScenario::Server => {
            writeln!(summary, "Scheduled samples per second : {qps:.6}")?;
            writeln!(summary, "Completed samples per second : {qps:.6}")?;
            for percentile in [50.0, 90.0, 95.0, 97.0, 99.0, 99.9] {
                if let Some(latency) = crate::stats::percentile_type7(&latencies, percentile) {
                    writeln!(
                        summary,
                        "{percentile:.2} percentile latency (ns) : {:.0}",
                        latency * 1e9
                    )?;
                }
            }
        }
        MlperfScenario::Offline => {
            writeln!(summary, "Samples per second : {qps:.6}")?;
        }
    }
    writeln!(summary, "Test Parameters Used")?;
    writeln!(summary, "samples_per_query : {}", records.len())?;
    writeln!(summary, "duration (s) : {duration_s:.6}")?;
    writeln!(
        summary,
        "Result validity : {}",
        if !records.is_empty() && completed == records.len() {
            "UNOFFICIAL_OK (see disclaimer; not an official MLPerf VALID)"
        } else {
            "UNOFFICIAL_INVALID (see disclaimer)"
        }
    )?;
    let mut detail = File::create(directory.join("mlperf_log_detail.txt"))?;
    writeln!(detail, "{DISCLAIMER}")?;
    for record in records {
        writeln!(
            detail,
            ":::MLLOG {{\"key\":\"sample\",\"value\":{{\"id\":{},\"scheduled_time_ns\":{},\"sent_time_ns\":{},\"latency_ns\":{:.0},\"success\":{}}},\"metadata\":{{\"file\":\"metrumbench\",\"lineno\":0}}}}",
            record.seq,
            record.scheduled_unix_ns,
            record.sent_unix_ns,
            record.latency_s * 1e9,
            record.success
        )?;
    }
    let accuracy = serde_json::json!({
        "disclaimer": DISCLAIMER,
        "accuracy": []
    });
    File::create(directory.join("mlperf_log_accuracy.json"))?
        .write_all(format!("{accuracy}\n").as_bytes())?;
    Ok(())
}

/// Sends OTLP/HTTP JSON mappings. Compiled only when explicitly enabled so
/// normal installations have no telemetry behavior.
#[cfg(feature = "otlp")]
pub async fn export_otlp(
    client: &reqwest::Client,
    endpoint: &str,
    service_name: &str,
    records: &[BenchRecord],
    headers: &BTreeMap<String, String>,
) -> Result<()> {
    let spans: Vec<_> = records
        .iter()
        .map(|record| {
            let trace_id = format!("{:032x}", record.scheduled_unix_ns ^ record.seq as u128);
            let span_id = format!("{:016x}", (record.sent_unix_ns as u64) ^ record.seq);
            json!({
                "traceId": trace_id, "spanId": span_id, "name": "inference.request",
                "kind": 3, "startTimeUnixNano": record.scheduled_unix_ns.to_string(),
                "endTimeUnixNano": (record.scheduled_unix_ns + (record.latency_s * 1e9) as u128).to_string(),
                "attributes": [
                    {"key":"benchmark.stage","value":{"doubleValue":record.stage}},
                    {"key":"benchmark.success","value":{"boolValue":record.success}},
                    {"key":"benchmark.queue_delay_s","value":{"doubleValue":record.queue_delay_s}},
                    {"key":"benchmark.service_latency_s","value":{"doubleValue":record.service_latency_s}},
                    {"key":"server.address","value":{"stringValue":record.endpoint}}
                ],
                "status":{"code": if record.success { 1 } else { 2 }}
            })
        })
        .collect();
    let body = json!({"resourceSpans":[{
        "resource":{"attributes":[{"key":"service.name","value":{"stringValue":service_name}}]},
        "scopeSpans":[{"scope":{"name":"metrumbench"},"spans":spans}]
    }]});
    let url = format!("{}/v1/traces", endpoint.trim_end_matches('/'));
    let mut request = client.post(url).json(&body);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    request.send().await?.error_for_status()?;
    let now = now_unix_ns().to_string();
    let successes = records.iter().filter(|record| record.success).count();
    let latency_values = crate::stats::sort_finite(
        records
            .iter()
            .filter(|record| record.success)
            .map(|record| record.latency_s),
    );
    let mut metrics = vec![
        json!({"name":"benchmark.requests","unit":"{request}","sum":{
            "aggregationTemporality":2,"isMonotonic":true,
            "dataPoints":[{"asInt":records.len().to_string(),"timeUnixNano":now}]
        }}),
        json!({"name":"benchmark.successes","unit":"{request}","sum":{
            "aggregationTemporality":2,"isMonotonic":true,
            "dataPoints":[{"asInt":successes.to_string(),"timeUnixNano":now}]
        }}),
    ];
    if let Some(p95) = crate::stats::percentile_type7(&latency_values, 95.0) {
        metrics.push(json!({"name":"benchmark.latency.p95","unit":"s","gauge":{
            "dataPoints":[{"asDouble":p95,"timeUnixNano":now}]
        }}));
    }
    let metric_body = json!({"resourceMetrics":[{
        "resource":{"attributes":[{"key":"service.name","value":{"stringValue":service_name}}]},
        "scopeMetrics":[{"scope":{"name":"metrumbench"},"metrics":metrics}]
    }]});
    let metrics_url = format!("{}/v1/metrics", endpoint.trim_end_matches('/'));
    let mut request = client.post(metrics_url).json(&metric_body);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    request.send().await?.error_for_status()?;
    Ok(())
}

pub fn now_unix_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knee_finds_curve_bend() {
        let points: Vec<_> = [(1.0, 1.0), (2.0, 1.1), (3.0, 1.3), (3.2, 4.0)]
            .into_iter()
            .map(|(throughput, latency)| {
                let latency_s = crate::stats::DistSummary::from_values(&[latency]);
                SweepPoint {
                    load: throughput,
                    n: 1,
                    errors: 0,
                    throughput,
                    latency_s: latency_s.clone(),
                    p50_s: Some(latency),
                    p95_s: Some(latency),
                    p99_s: Some(latency),
                    p99_unreliable: latency_s.p99_unreliable,
                    error_rate: Some(0.0),
                    validity_rate: None,
                    goodput: throughput,
                    goodput_equals_throughput: true,
                    slo_thresholds_s: None,
                    config: None,
                }
            })
            .collect();
        assert_eq!(detect_knee(&points), Some(2));
    }

    #[test]
    fn parses_vllm_metrics() {
        let metrics = parse_server_metrics(
            "vllm:gpu_cache_usage_perc 0.75\nvllm:num_requests_waiting 3\nvllm:num_preemptions_total 2\n",
        );
        assert_eq!(metrics.kv_cache_usage, Some(0.75));
        assert_eq!(metrics.requests_waiting, Some(3.0));
    }

    #[test]
    fn validates_json_required_properties() {
        let validator = Validity::json_schema(&json!({
            "type":"object",
            "properties":{"answer":{"type":"integer"}},
            "required":["answer"],
            "additionalProperties":false
        }))
        .expect("valid schema");
        assert!(validator.validate(&json!({
            "choices":[{"message":{"content":"{\"answer\":42}"}}]
        })));
        assert!(!validator.validate(&json!({
            "choices":[{"message":{"content":"{}"}}]
        })));
        assert!(!validator.validate(&json!({
            "choices":[{"message":{"content":"{\"answer\":\"wrong type\"}"}}]
        })));
    }

    #[test]
    fn validates_tool_name_and_argument_schema() {
        let validator = Validity::tool_names(&json!([{
            "type":"function",
            "function":{
                "name":"lookup",
                "parameters":{
                    "type":"object",
                    "properties":{"id":{"type":"integer"}},
                    "required":["id"]
                }
            }
        }]))
        .expect("valid tools");
        let response = |name, arguments| {
            json!({"choices":[{"message":{"tool_calls":[{
                "function":{"name":name,"arguments":arguments}
            }]}}]})
        };
        assert!(validator.validate(&response("lookup", r#"{"id":1}"#)));
        assert!(!validator.validate(&response("lookup", r#"{"id":"1"}"#)));
        assert!(!validator.validate(&response("other", r#"{"id":1}"#)));
    }

    #[test]
    fn unique_prefix_is_session_scoped() {
        let messages = vec![json!({"role":"user","content":"hello"})];
        let controlled =
            controlled_messages(&messages, Some("common"), PrefixControl::Unique, "s1");
        assert_eq!(controlled[0]["content"], "[session:s1] common");
    }

    #[test]
    fn sweep_uses_type7_and_null_for_undefined_latency() {
        let record = |seq, latency_s| BenchRecord {
            seq,
            stage: 1.0,
            endpoint: "http://example.test".to_string(),
            scheduled_unix_ns: 1,
            sent_unix_ns: 1,
            latency_s,
            queue_delay_s: 0.0,
            service_latency_s: latency_s,
            success: true,
            valid: None,
            input_tokens: 0,
            output_tokens: 0,
            session_id: None,
            turn: None,
            error: None,
        };
        let point = summarize_stage(
            1.0,
            &[
                record(0, 1.0),
                record(1, 2.0),
                record(2, 3.0),
                record(3, 4.0),
            ],
            1.0,
            &crate::summary::SloConfig::default(),
            None,
        );
        assert_eq!(point.n, 4);
        assert_eq!(point.errors, 0);
        assert!(point.goodput_equals_throughput);
        assert!((point.p95_s.expect("p95") - 3.85).abs() < 1e-12);

        let empty = summarize_stage(1.0, &[], 1.0, &crate::summary::SloConfig::default(), None);
        let value = serde_json::to_value(empty).expect("serialize sweep point");
        assert!(value["p50_s"].is_null());
        assert!(value["p95_s"].is_null());
        assert!(value["p99_s"].is_null());
        assert!(value["error_rate"].is_null());
    }

    #[test]
    fn mlperf_export_uses_scenario_fields_and_scheduled_latency() {
        let directory = tempfile::tempdir().expect("temporary MLPerf directory");
        let record = BenchRecord {
            seq: 7,
            stage: 10.0,
            endpoint: "http://example.test".to_string(),
            scheduled_unix_ns: 100,
            sent_unix_ns: 120,
            latency_s: 0.25,
            queue_delay_s: 0.02,
            service_latency_s: 0.23,
            success: true,
            valid: None,
            input_tokens: 1,
            output_tokens: 1,
            session_id: None,
            turn: None,
            error: None,
        };
        export_mlperf(directory.path(), MlperfScenario::Server, &[record], 1.0)
            .expect("export MLPerf logs");
        let summary = std::fs::read_to_string(directory.path().join("mlperf_log_summary.txt"))
            .expect("summary log");
        assert!(summary.contains("Scenario : Server"));
        assert!(summary.contains("90.00 percentile latency (ns) : 250000000"));
        assert!(summary.contains("UNOFFICIAL"));
        assert!(summary.contains("Result validity : UNOFFICIAL_OK"));
        assert!(
            !summary.contains("Result is : VALID"),
            "must not emit the official LoadGen VALID substring"
        );
        let detail = std::fs::read_to_string(directory.path().join("mlperf_log_detail.txt"))
            .expect("detail log");
        assert!(detail.starts_with("UNOFFICIAL"));
        assert!(detail.contains("\"scheduled_time_ns\":100"));
        assert!(detail.contains("\"sent_time_ns\":120"));
    }

    #[cfg(feature = "otlp")]
    #[tokio::test]
    async fn otlp_exports_traces_and_metrics() {
        use axum::extract::{OriginalUri, State};
        use axum::routing::post;
        use axum::{Json, Router};
        use std::sync::Arc;
        use tokio::sync::Mutex;

        type CapturedCalls = Arc<Mutex<Vec<(String, Value)>>>;

        async fn capture(
            State(calls): State<CapturedCalls>,
            OriginalUri(uri): OriginalUri,
            Json(body): Json<Value>,
        ) {
            calls.lock().await.push((uri.path().to_string(), body));
        }

        let calls = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route("/v1/traces", post(capture))
            .route("/v1/metrics", post(capture))
            .with_state(calls.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind collector");
        let address = listener.local_addr().expect("collector address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve collector");
        });
        let record = BenchRecord {
            seq: 1,
            stage: 5.0,
            endpoint: "http://example.test".to_string(),
            scheduled_unix_ns: 100,
            sent_unix_ns: 110,
            latency_s: 0.5,
            queue_delay_s: 0.1,
            service_latency_s: 0.4,
            success: true,
            valid: Some(true),
            input_tokens: 2,
            output_tokens: 3,
            session_id: None,
            turn: None,
            error: None,
        };
        export_otlp(
            &reqwest::Client::new(),
            &format!("http://{address}"),
            "test",
            &[record],
            &BTreeMap::new(),
        )
        .await
        .expect("OTLP export");
        server.abort();
        let calls = calls.lock().await;
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "/v1/traces");
        assert_eq!(calls[1].0, "/v1/metrics");
        assert_eq!(
            calls[0]
                .1
                .pointer("/resourceSpans/0/scopeSpans/0/spans/0/startTimeUnixNano"),
            Some(&json!("100"))
        );
        assert_eq!(
            calls[1]
                .1
                .pointer("/resourceMetrics/0/scopeMetrics/0/metrics/0/name"),
            Some(&json!("benchmark.requests"))
        );
    }
}
