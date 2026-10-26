// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Endpoints-compatible, explicitly non-certifying artifact adapter (#306).
//!
//! Converts a completed bench run data log into an Endpoints-shaped bundle
//! (`point.yaml`, `system_desc.json`, `system_power.json`,
//! `performance/result_summary.json`, `config.yaml`, `crosscheck.json`,
//! `NONCOMPLIANCE.md`) with explicit units, provenance, model/workload/point
//! metadata and per-metric comparability flags.
//!
//! The bundle is parseable by the upstream submission-checker's Endpoints
//! result parser (field names and layouts mirror
//! `tools/submission/submission_checker/parsers/endpoints_parser.py`), but
//! it is NEVER an MLPerf Endpoints submission: no accuracy/coverage
//! evidence, no config-locked client, and native timing definitions differ
//! where flagged. Full submission validation against the pinned upstream
//! checker commit is impossible locally (see NONCOMPLIANCE.md).

use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Banner carried by every emitted file.
pub const DISCLAIMER: &str = "EXPLICITLY NON-CERTIFYING: this bundle was produced by Metrum AI Bench CLI's Endpoints-compatible artifact adapter. It is NOT an MLPerf or Endpoints submission, and bench-native metric definitions (visible-token TTFT, per-request TPOT, closed-loop arrival) must not be read as Endpoints metric definitions without a definition review.";

/// Adapter bundle schema version.
pub const ADAPTER_SCHEMA_VERSION: &str = "metrum-ai-bench-cli.endpoints-adapter.v1";

/// Operator-declared Endpoints scenario intent for the point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize)]
#[value(rename_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum ScenarioKind {
    /// Closed-loop / paced online point (native for bench).
    Server,
    /// Population-burst point; requires a native Offline burst mode (#309
    /// is not shipped), so the adapter always discloses the mapping.
    Offline,
    /// LoadStair is not a bench arrival pattern: emitted with an explicit
    /// unsupported-scenario disclosure, never silently.
    Loadstair,
    /// Pareto is not a bench arrival pattern: same disclosure.
    Pareto,
}

/// Inputs for the adapter (all operator-declared, never invented).
pub struct ExportOptions {
    /// Bench run data log (JSONL with request rows + summary rows).
    pub data_log: PathBuf,
    /// Output directory for the bundle.
    pub out_dir: PathBuf,
    /// Optional operator-declared SUT file (drives system_desc.json).
    pub sut: Option<PathBuf>,
    /// Operator workload label (e.g. the campaign's W1/W2 name).
    pub workload: String,
    /// Operator's scenario intent.
    pub scenario: ScenarioKind,
    /// Optional operator-declared power descriptor file (free-form JSON,
    /// passed through with its sha256 recorded). When absent,
    /// system_power.json records `declared: false`; power facts are never
    /// synthesized.
    pub power_desc: Option<PathBuf>,
    /// Free-form operator notes.
    pub notes: Option<String>,
}

/// What the adapter wrote, for the CLI summary print.
#[derive(Debug)]
pub struct BundleSummary {
    pub files: Vec<String>,
    pub run_id: String,
    pub sample_count: usize,
    pub scenario_disclosure: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
struct BenchRun {
    run_id: String,
    summary: Value,
    /// measure-phase request rows belonging to the run.
    records: Vec<Value>,
    /// Total request rows of the run (warmup included).
    total_records: usize,
}

/// Select the complete run from a data log, mirroring the study extractor:
/// exactly one complete (non-partial) summary with matching request rows.
/// Ambiguity, missing summaries and partial-only logs are hard errors —
/// never manufacture a point from a broken run.
fn select_complete_run(rows: &[Value]) -> Result<BenchRun> {
    let summaries: Vec<&Value> = rows
        .iter()
        .filter(|r| {
            r.get("schema_version")
                .and_then(Value::as_str)
                .is_some_and(|v| v.contains(".summary."))
        })
        .collect();
    let complete: Vec<&Value> = summaries
        .iter()
        .filter(|s| !s.get("partial").and_then(Value::as_bool).unwrap_or(false))
        .copied()
        .collect();
    if complete.len() > 1 {
        bail!("ambiguous data log: multiple complete summaries; isolate the intended run before exporting");
    }
    if !summaries.is_empty() && complete.is_empty() {
        bail!("data log has only partial summaries; no complete run to export");
    }
    let Some(summary) = complete.first().copied() else {
        bail!("data log has no summary row; export a completed run");
    };
    let run_id = summary
        .get("run_id")
        .or_else(|| summary.pointer("/config/run_id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let requests: Vec<Value> = rows
        .iter()
        .filter(|r| {
            r.get("schema_version")
                .and_then(Value::as_str)
                .is_some_and(|v| v.contains(".request."))
        })
        .cloned()
        .collect();
    let selected: Vec<Value> = if run_id.is_empty() {
        let ids: std::collections::BTreeSet<String> = requests
            .iter()
            .filter_map(|r| r.get("run_id").and_then(Value::as_str).map(String::from))
            .collect();
        if ids.len() > 1 {
            bail!("ambiguous data log: multiple request run_ids without a selectable complete summary");
        }
        requests
    } else {
        requests
            .into_iter()
            .filter(|r| r.get("run_id").and_then(Value::as_str) == Some(run_id.as_str()))
            .collect()
    };
    if selected.is_empty() {
        bail!("complete summary has no matching request rows (run_id {run_id})");
    }
    let measure: Vec<Value> = selected
        .iter()
        .filter(|r| r.get("phase").and_then(Value::as_str) == Some("measure"))
        .cloned()
        .collect();
    let total_records = selected.len();
    Ok(BenchRun {
        run_id,
        summary: summary.clone(),
        records: measure,
        total_records,
    })
}

fn f64_field(row: &Value, key: &str) -> Option<f64> {
    row.get(key).and_then(Value::as_f64)
}

/// Native bench metrics for one point, each with unit, definition and a
/// comparability note. Summary-derived vs record-derived values are
/// cross-checked; a mismatch flags the metric instead of silently
/// shipping it.
#[derive(Debug, Default)]
struct NativeMetrics {
    values: BTreeMap<String, Value>,
    /// metric -> (summary value, record value, match)
    crosscheck: Vec<(String, Option<f64>, Option<f64>, bool)>,
}

impl NativeMetrics {
    fn put(
        &mut self,
        name: &str,
        value: Option<f64>,
        unit: &str,
        definition: &str,
        comparability: &str,
        summary_value: Option<f64>,
    ) {
        let record_value = value;
        let match_flag = match (record_value, summary_value) {
            (Some(a), Some(b)) => (a - b).abs() <= 1e-6 * a.abs().max(1.0),
            _ => true,
        };
        self.crosscheck
            .push((name.to_string(), record_value, summary_value, match_flag));
        self.values.insert(
            name.to_string(),
            json!({
                "value": value,
                "unit": unit,
                "definition": definition,
                "comparability": comparability,
            }),
        );
    }
}

/// Compute the native metrics from the selected measure-phase records.
fn native_metrics(run: &BenchRun) -> Result<NativeMetrics> {
    let summary = &run.summary;
    let records = &run.records;
    if records.is_empty() {
        bail!("no measure-phase request rows for run {}", run.run_id);
    }
    let ok: Vec<&Value> = records
        .iter()
        .filter(|r| r.get("error").is_none_or(Value::is_null))
        .collect();
    let attempted = records.len();
    let failed = attempted - ok.len();
    let error_rate = failed as f64 / attempted.max(1) as f64;

    // Window: the complete summary's window; falling back to the records'
    // own span is disclosed in the definition.
    let summary_window = f64_field(summary, "window_seconds");
    let window_s = match summary_window {
        Some(w) if w > 0.0 => w,
        _ => {
            let start = records
                .iter()
                .filter_map(|r| {
                    r.get("started_at")
                        .and_then(Value::as_str)
                        .and_then(parse_iso_epoch)
                })
                .fold(f64::MAX, f64::min);
            let stop = records
                .iter()
                .filter_map(|r| {
                    r.get("completed_at")
                        .and_then(Value::as_str)
                        .and_then(parse_iso_epoch)
                })
                .fold(f64::MIN, f64::max);
            if start.is_finite() && stop.is_finite() && stop > start {
                stop - start
            } else {
                bail!("cannot derive a duration window: summary.window_seconds is missing and record timestamps are unusable");
            }
        }
    };

    let mut out = NativeMetrics::default();
    let requests_s = ok.len() as f64 / window_s;
    out.put(
        "requests_s",
        Some(requests_s),
        "requests/s",
        "successful measure-phase requests divided by the run window",
        "native-arithmetic",
        summary
            .pointer("/window_requests_per_second")
            .and_then(Value::as_f64),
    );
    out.put(
        "error_rate",
        Some(error_rate),
        "fraction",
        "failed measure-phase requests divided by all measure-phase requests",
        "native-arithmetic",
        summary.pointer("/error_rate").and_then(Value::as_f64),
    );

    // Token rates: only when every successful row reports usage
    // (usage_missing rows would silently undercount; flagged otherwise).
    let usage_missing = ok
        .iter()
        .filter(|r| {
            r.get("usage_missing")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .count();
    let usage_complete = ok.iter().all(|r| {
        !r.get("usage_missing")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    });
    let output_tokens: Option<f64> = if usage_complete {
        Some(
            ok.iter()
                .filter_map(|r| f64_field(r, "completion_tokens"))
                .sum(),
        )
    } else {
        None
    };
    let input_tokens: Option<f64> = if usage_complete {
        Some(
            ok.iter()
                .filter_map(|r| f64_field(r, "prompt_tokens"))
                .sum(),
        )
    } else {
        None
    };
    let completion_per_second = output_tokens.map(|t| t / window_s);
    out.put(
        "system_output_tok_s",
        completion_per_second,
        "output tokens/s",
        "sum of server-reported completion tokens over successful requests, divided by the run window; absent when any success is usage_missing",
        "native-arithmetic (usage-complete only)",
        summary
            .pointer("/completion_tokens_per_second")
            .and_then(Value::as_f64),
    );
    out.put(
        "total_tok_s",
        input_tokens.and_then(|i| output_tokens.map(|o| (i + o) / window_s)),
        "tokens/s",
        "sum of prompt + completion tokens over successful requests, divided by the run window; absent when any success is usage_missing",
        "native-arithmetic (usage-complete only)",
        None,
    );
    if usage_missing > 0 {
        out.values.insert(
            "usage_missing_requests".into(),
            json!({"value": usage_missing, "unit": "requests", "definition": "successful rows without server token usage; token rates are dropped instead of undercounted", "comparability": "coverage"}),
        );
    }

    // Latency percentiles: E2E plus TTFT (visible token) and per-request
    // TPOT, all type-7 percentiles of finite values.
    let latency: Vec<f64> = ok
        .iter()
        .filter_map(|r| f64_field(r, "latency_s"))
        .collect();
    let latency = crate::stats::sort_finite(latency);
    let ttft: Vec<f64> = ok.iter().filter_map(|r| f64_field(r, "ttft_s")).collect();
    let ttft = crate::stats::sort_finite(ttft);
    let mut tpot: Vec<f64> = Vec::new();
    for r in &ok {
        let (n, t, lat) = (
            f64_field(r, "completion_tokens"),
            f64_field(r, "ttft_s"),
            f64_field(r, "latency_s"),
        );
        if let (Some(n), Some(ttft), Some(lat)) = (n, t, lat) {
            if n > 1.0 && lat > ttft {
                tpot.push((lat - ttft) / (n - 1.0));
            }
        }
    }
    let tpot = crate::stats::sort_finite(tpot);

    let percentile_ms =
        |sorted: &[f64], p: f64| crate::stats::percentile_type7(sorted, p).map(|v| v * 1000.0);
    for (p, name) in [
        (50.0, "ttft_p50_ms"),
        (90.0, "ttft_p90_ms"),
        (99.0, "ttft_p99_ms"),
    ] {
        out.put(
            name,
            percentile_ms(&ttft, p),
            "ms",
            "type-7 percentile of visible-token TTFT (first content token, not first byte)",
            "flag-definition: bench TTFT is first visible content token; Endpoints TTFT semantics differ — review before comparing",
            summary
                .pointer(&format!("/ttft_s/p{:.0}", p as u64))
                .and_then(Value::as_f64)
                .map(|v| v * 1000.0),
        );
    }
    for (p, name) in [(50.0, "tpot_p50_ms"), (90.0, "tpot_p90_ms")] {
        out.put(
            name,
            percentile_ms(&tpot, p),
            "ms",
            "type-7 percentile of per-request (latency − ttft) / (completion_tokens − 1)",
            "flag-definition: per-request TPOT from bench records; Endpoints TPOT is a steady-state aggregate — review before comparing",
            None,
        );
    }
    for (p, name) in [
        (50.0, "e2e_p50_ms"),
        (90.0, "e2e_p90_ms"),
        (99.0, "e2e_p99_ms"),
    ] {
        out.put(
            name,
            percentile_ms(&latency, p),
            "ms",
            "type-7 percentile of send-to-completion latency",
            "native-arithmetic",
            None,
        );
    }
    out.values.insert(
        "sample_count".into(),
        json!({"value": ok.len(), "unit": "requests", "definition": "successful measure-phase requests", "comparability": "native-arithmetic"}),
    );
    Ok(out)
}

fn parse_iso_epoch(value: &str) -> Option<f64> {
    let dt = chrono::DateTime::parse_from_rfc3339(value).ok()?;
    Some(dt.timestamp() as f64 + f64::from(dt.timestamp_subsec_nanos()) / 1e9)
}

/// Emit the bundle. Returns the written file list.
pub fn export_bundle(options: &ExportOptions) -> Result<BundleSummary> {
    let raw = std::fs::read_to_string(&options.data_log)
        .with_context(|| format!("read data log {}", options.data_log.display()))?;
    let rows: Vec<Value> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    let run = select_complete_run(&rows)?;
    let metrics = native_metrics(&run)?;

    std::fs::create_dir_all(options.out_dir.join("performance"))?;
    std::fs::create_dir_all(options.out_dir.join("accuracy"))?;

    let mut warnings: Vec<String> = Vec::new();
    let mut files: Vec<String> = Vec::new();

    // Scenario mapping disclosure: bench has no LoadStair/Pareto arrival
    // process, and no native Offline burst mode yet (#309).
    // Arrival can be stamped flat (config.arrival) or nested in the
    // effective common args (config.common.arrival); both are the raw
    // --arrival value ("burst", "constant", "poisson"), defaulting to the
    // closed-loop absence.
    let arrival = run
        .summary
        .pointer("/config/arrival")
        .or_else(|| run.summary.pointer("/config/common/arrival"))
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase)
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| "closed-loop".to_string());
    let (upstream_type, scenario_disclosure) = match options.scenario {
        ScenarioKind::Server => ("Server", None),
        ScenarioKind::Offline => {
            // #329×#330 follow-up: a burst-arrival run is the native
            // Offline-style mode (#309); anything else stays a disclosed
            // proxy.
            if arrival == "burst" {
                ("Offline", Some(
                    "native burst (bench definition; not an accepted MLPerf Offline result)"
                        .to_string(),
                ))
            } else {
                ("Offline", Some(format!(
                    "Declared Offline on a bench '{arrival}' run: bench has no native Offline burst mode (#309). This point is a closed-loop/fixed-rate run exported with an Offline intent label; it is not a native Offline arrival process."
                )))
            }
        }
        ScenarioKind::Loadstair => (
            "LoadStair",
            Some(
                "LoadStair is not a bench load pattern; exported with an explicit unsupported-scenario disclosure. No mapping to a bench arrival process exists."
                    .to_string(),
            ),
        ),
        ScenarioKind::Pareto => (
            "Pareto",
            Some(
                "Pareto is not a bench load pattern; exported with an explicit unsupported-scenario disclosure. No mapping to a bench arrival process exists."
                    .to_string(),
            ),
        ),
    };

    // ---- point.yaml: units, provenance, point metadata ----
    let config_model = run.summary.pointer("/config/model").and_then(Value::as_str);
    let sut = match &options.sut {
        Some(path) => Some(crate::sut::load_sut(path)?),
        None => None,
    };
    let model_id = sut
        .as_ref()
        .and_then(|s| s.model.as_ref())
        .and_then(|m| m.id.clone())
        .or_else(|| config_model.map(String::from));
    let tool_version = run
        .summary
        .pointer("/tool_version")
        .or_else(|| run.summary.pointer("/config/tool_version"))
        .and_then(Value::as_str)
        .unwrap_or(env!("CARGO_PKG_VERSION"))
        .to_string();

    let mut point_yaml = format!(
        "# {DISCLAIMER}\nschema_version: {ADAPTER_SCHEMA_VERSION}\npoint:\n  tool: metrum-ai-bench-cli\n  tool_version: \"{tool_version}\"\n  run_id: \"{}\"\n  model: {}\n  workload: \"{}\"\n  scenario_declared: {upstream_type}\n  arrival_observed: \"{arrival}\"\n",
        run.run_id,
        model_id
            .as_deref()
            .map(|m| format!("\"{m}\""))
            .unwrap_or_else(|| "null".into()),
        options.workload,
    );
    if let Some(disclosure) = &scenario_disclosure {
        point_yaml.push_str(&format!("  scenario_disclosure: {disclosure:?}\n"));
    }
    if let Some(notes) = &options.notes {
        point_yaml.push_str(&format!("  operator_notes: {notes:?}\n"));
    }
    point_yaml.push_str("  units:\n    latency: ms\n    throughput: output tokens/s or requests/s per metric definition\n    power: W (board power only, declared descriptors only)\n  provenance: bench data log converted by the adapter; see NONCOMPLIANCE.md for the checks that remain impossible\n");
    write_file(&options.out_dir.join("point.yaml"), point_yaml)?;
    files.push("point.yaml".into());

    // ---- system_desc.json: declared-only SUT subset ----
    let system_desc = match &sut {
        Some(sut) => json!({
            "schema_version": ADAPTER_SCHEMA_VERSION,
            "disclaimer": DISCLAIMER,
            "declared_only": true,
            "gpu": sut.gpu,
            "driver_version": sut.driver_version,
            "runtime": sut.runtime,
            "model": sut.model,
            "host_os": sut.host_os,
            "cost": sut.cost,
        }),
        None => json!({
            "schema_version": ADAPTER_SCHEMA_VERSION,
            "disclaimer": DISCLAIMER,
            "declared_only": true,
            "missing": true,
            "reason": "no --sut file provided; nothing is synthesized",
        }),
    };
    write_json(&options.out_dir.join("system_desc.json"), &system_desc)?;
    files.push("system_desc.json".into());

    // ---- system_power.json: declared power descriptors only ----
    let system_power = match &options.power_desc {
        Some(path) => {
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("read power descriptor {}", path.display()))?;
            let value: Value = serde_json::from_str(&raw)
                .with_context(|| format!("parse power descriptor {}", path.display()))?;
            let digest = {
                use sha2::Digest;
                let mut h = sha2::Sha256::new();
                h.update(raw.as_bytes());
                h.finalize()
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            };
            json!({
                "schema_version": ADAPTER_SCHEMA_VERSION,
                "disclaimer": DISCLAIMER,
                "declared": true,
                "source_file_hash_sha256": digest,
                "descriptors": value,
            })
        }
        None => json!({
            "schema_version": ADAPTER_SCHEMA_VERSION,
            "disclaimer": DISCLAIMER,
            "declared": false,
            "reason": "no operator-declared power descriptors; PDU/whole-system facts are never synthesized by the adapter",
        }),
    };
    write_json(&options.out_dir.join("system_power.json"), &system_power)?;
    files.push("system_power.json".into());

    // ---- performance/result_summary.json: upstream-parser-compatible keys ----
    let window_s = {
        // Recompute window for the file (same rule as native_metrics).
        f64_field(&run.summary, "window_seconds").unwrap_or(0.0)
    };
    let metric = |name: &str| {
        metrics
            .values
            .get(name)
            .and_then(|m| m["value"].clone().as_f64())
    };
    let ttft_bucket = |pct: f64| {
        metrics
            .values
            .get(&format!("ttft_p{:.0}_ms", pct as u64))
            .and_then(|m| m["value"].clone().as_f64())
    };
    let tpot_bucket = |pct: f64| {
        metrics
            .values
            .get(&format!("tpot_p{:.0}_ms", pct as u64))
            .and_then(|m| m["value"].clone().as_f64())
    };
    let e2e = |pct: f64| {
        metrics
            .values
            .get(&format!("e2e_p{:.0}_ms", pct as u64))
            .and_then(|m| m["value"].clone().as_f64())
    };
    let ok_count = metrics.values["sample_count"]["value"]
        .as_f64()
        .unwrap_or(0.0) as u64;
    let attempted = run.records.len() as u64;
    let failed_count = attempted.saturating_sub(ok_count);

    let result_summary = json!({
        "disclaimer": DISCLAIMER,
        "qps": metric("requests_s"),
        "warmup_rows": run.total_records - run.records.len(),
        "n_samples_issued": attempted,
        "n_samples_completed": ok_count,
        "n_samples_failed": failed_count,
        "duration_ns": (window_s * 1e9) as u64,
        "test_datetime": run.summary.get("started_at").and_then(Value::as_str).unwrap_or(""),
        "git_sha": tool_version,
        "result_completed_tokens_per_second": metric("system_output_tok_s"),
        "result_scheduled_samples_per_sec": metric("requests_s"),
        "latency": {
            "early_stopping_percentiles": {
                "50.0": e2e(50.0).map(|v| v * 1e6),
                "90.0": e2e(90.0).map(|v| v * 1e6),
                "99.0": e2e(99.0).map(|v| v * 1e6),
            },
            "bench_definition": "E2E send-to-completion latency, ns",
        },
        "ttft": {
            "early_stopping_percentiles": {
                "50.0": ttft_bucket(50.0).map(|v| v * 1e6),
                "90.0": ttft_bucket(90.0).map(|v| v * 1e6),
                "99.0": ttft_bucket(99.0).map(|v| v * 1e6),
            },
            "bench_definition": "bench visible-token TTFT (first content token), ns; NOT the first-byte clock; do not equate with Endpoints TTFT without a definition review",
        },
        "tpot": {
            "early_stopping_percentiles": {
                "50.0": tpot_bucket(50.0).map(|v| v * 1e6),
                "90.0": tpot_bucket(90.0).map(|v| v * 1e6),
            },
            "bench_definition": "per-request (latency - ttft) / (completion_tokens - 1) percentile, ns; not a steady-state aggregate",
        },
    });
    write_json(
        &options
            .out_dir
            .join("performance")
            .join("result_summary.json"),
        &result_summary,
    )?;
    files.push("performance/result_summary.json".into());

    // ---- config.yaml: scenario identity, never an upstream version ----
    let streaming = run
        .summary
        .pointer("/config/streaming")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let concurrency = run
        .summary
        .pointer("/config/concurrency")
        .and_then(Value::as_u64);
    let request_rate = run
        .summary
        .pointer("/config/request_rate")
        .and_then(Value::as_f64);
    let mut config_yaml = format!(
        "# {DISCLAIMER}\nversion: \"{ADAPTER_SCHEMA_VERSION}\"\ntype: {upstream_type}\nmodel_params:\n  streaming: {streaming}\nsettings:\n  load_pattern:\n    type: \"{arrival}\"\n"
    );
    if let Some(c) = concurrency {
        config_yaml.push_str(&format!("    concurrency: {c}\n"));
    }
    if let Some(r) = request_rate {
        config_yaml.push_str(&format!("    target_qps: {r}\n"));
    }
    if let Some(disclosure) = &scenario_disclosure {
        config_yaml.push_str(&format!("    disclosure: {disclosure:?}\n"));
    }
    write_file(&options.out_dir.join("config.yaml"), config_yaml)?;
    files.push("config.yaml".into());

    // ---- accuracy/: never fabricated; a marker file states the gap ----
    write_file(
        &options.out_dir.join("accuracy").join("NOT_PROVIDED"),
        format!(
            "{DISCLAIMER}\n\nThe adapter never fabricates accuracy/coverage evidence.\nA real Endpoints submission needs its accuracy run; this directory is intentionally empty.\n"
        ),
    )?;
    files.push("accuracy/NOT_PROVIDED".into());

    // ---- crosscheck.json: summary-derived vs record-derived arithmetic ----
    let checks: Vec<Value> = metrics
        .crosscheck
        .iter()
        .filter(|(name, _, _, _)| name != "sample_count")
        .map(|(name, record_value, summary_value, matched)| {
            json!({
                "metric": name,
                "record_derived": record_value,
                "summary_derived": summary_value,
                "match": matched,
            })
        })
        .collect();
    let mismatches: Vec<String> = metrics
        .crosscheck
        .iter()
        .filter(|(_, _, _, matched)| !matched)
        .map(|(n, _, _, _)| n.clone())
        .collect();
    if !mismatches.is_empty() {
        warnings.push(format!(
            "crosscheck mismatches (record-derived vs summary-derived): {}",
            mismatches.join(", ")
        ));
    }
    let crosscheck = json!({
        "schema_version": ADAPTER_SCHEMA_VERSION,
        "disclaimer": DISCLAIMER,
        "checker_commit": "unavailable",
        "scope": "Adapter-internal cross-check only: record-derived vs summary-derived arithmetic. The study's upstream PointSummary cross-check (checker fda43ef8) is not publicly fetchable; supplying it is an operator/Phase-2 step.",
        "points": [{
            "tool": "metrum-ai-bench-cli",
            "model": model_id,
            "workload": options.workload,
            "native_or_adapter": "explicit adapter bundle; not a native export",
            "checks": checks,
        }],
    });
    write_json(&options.out_dir.join("crosscheck.json"), &crosscheck)?;
    files.push("crosscheck.json".into());

    // ---- NONCOMPLIANCE.md: which checks remain impossible ----
    write_file(
        &options.out_dir.join("NONCOMPLIANCE.md"),
        format!(
            "# {DISCLAIMER}\n\n\
            This bundle is parseable by the upstream Endpoints result parser, but the following submission checks remain IMPOSSIBLE from bench artifacts and are not attempted:\n\n\
            - Accuracy/coverage gating (no accuracy run exists in the bundle; the accuracy directory is empty by design).\n\
            - Config-locked client settings and LoadGen validity checks.\n\
            - The pinned upstream PointSummary cross-check (checker commit fda43ef88fffcef4e9da81df2b71dbb355dd78e8 is not publicly fetchable; supply it as an operator step and record results in crosscheck.json).\n\
            - Power Working Group evidence: system_power.json carries operator-declared descriptors only; PDU/whole-system facts are never synthesized.\n\
            - Scenario conformance: bench has no LoadStair/Pareto arrival pattern and no native Offline burst mode (#309); such intent labels are exported with an explicit disclosure, never as equivalent runs.\n\n\
            Metric definitions that differ from Endpoints: visible-token TTFT (first content token, not first byte) and per-request TPOT are flagged per metric in performance/result_summary.json and point.yaml.\n"
        ),
    )?;
    files.push("NONCOMPLIANCE.md".into());

    Ok(BundleSummary {
        files,
        run_id: run.run_id,
        sample_count: ok_count as usize,
        scenario_disclosure,
        warnings,
    })
}

fn write_file(path: &Path, body: String) -> Result<()> {
    std::fs::write(path, body).with_context(|| format!("write {}", path.display()))
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    let mut body = serde_json::to_string_pretty(value)?;
    body.push('\n');
    write_file(path, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Two successful + one failed measure-phase request rows and one
    /// complete summary for run-1.
    fn run_rows() -> Vec<Value> {
        vec![
            json!({
                "schema_version": "metrum-ai-bench-cli.request.v1",
                "run_id": "run-1", "seq": 0, "phase": "measure",
                "started_at": "2026-01-01T00:00:00Z", "completed_at": "2026-01-01T00:00:01Z",
                "latency_s": 1.0, "ttft_s": 0.1, "prompt_tokens": 100, "completion_tokens": 10,
                "usage_missing": false, "error": null
            }),
            json!({
                "schema_version": "metrum-ai-bench-cli.request.v1",
                "run_id": "run-1", "seq": 1, "phase": "measure",
                "started_at": "2026-01-01T00:00:01Z", "completed_at": "2026-01-01T00:00:03Z",
                "latency_s": 2.0, "ttft_s": 0.2, "prompt_tokens": 100, "completion_tokens": 20,
                "usage_missing": false, "error": null
            }),
            json!({
                "schema_version": "metrum-ai-bench-cli.request.v1",
                "run_id": "run-1", "seq": 2, "phase": "measure",
                "started_at": "2026-01-01T00:00:03Z", "completed_at": "2026-01-01T00:00:04Z",
                "latency_s": 1.0, "ttft_s": null, "prompt_tokens": 0, "completion_tokens": 0,
                "usage_missing": false,
                "error": {"category": "http_status"}
            }),
            json!({
                "schema_version": "metrum-ai-bench-cli.summary.v3",
                "run_id": "run-1", "partial": false, "window_seconds": 10.0,
                "window_requests_per_second": 0.2,
                "ttft_s": {"p50": 0.15, "p90": 0.19},
                "completion_tokens_per_second": 3.0,
                "tool_version": "test",
                "config": {"model": "example/model", "concurrency": 2, "arrival": "closed-loop"}
            }),
        ]
    }

    fn write_log(dir: &Path, rows: &[Value]) -> PathBuf {
        let path = dir.join("run.jsonl");
        std::fs::write(
            &path,
            rows.iter()
                .map(|r| serde_json::to_string(r).unwrap())
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        path
    }

    fn export(
        dir: &Path,
        rows: &[Value],
        scenario: ScenarioKind,
        power: Option<PathBuf>,
    ) -> Result<BundleSummary> {
        let options = ExportOptions {
            data_log: write_log(dir, rows),
            out_dir: dir.join("bundle"),
            sut: None,
            workload: "W1".into(),
            scenario,
            power_desc: power,
            notes: None,
        };
        export_bundle(&options)
    }

    #[test]
    fn bundle_writes_all_files_with_disclaimers_and_golden_metrics() {
        let dir = tempfile::tempdir().unwrap();
        let summary = export(dir.path(), &run_rows(), ScenarioKind::Server, None).unwrap();
        assert_eq!(summary.run_id, "run-1");
        assert_eq!(summary.sample_count, 2);
        assert!(summary.scenario_disclosure.is_none());
        assert!(summary.warnings.is_empty(), "{:?}", summary.warnings);

        let result: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("bundle/performance/result_summary.json"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(result["n_samples_issued"], 3);
        assert_eq!(result["n_samples_completed"], 2);
        assert_eq!(result["n_samples_failed"], 1);
        assert_eq!(result["qps"], 0.2);
        assert_eq!(result["result_completed_tokens_per_second"], 3.0);
        // ttft p50 (type-7 interpolation) of [0.1, 0.2]s → 0.15s = 150e6 ns
        // (float epsilon: compare with tolerance).
        let p50_ns = result["ttft"]["early_stopping_percentiles"]["50.0"]
            .as_f64()
            .unwrap();
        assert!((p50_ns - 150_000_000.0).abs() < 1.0, "{p50_ns}");
        assert!(result["ttft"]["bench_definition"]
            .as_str()
            .unwrap()
            .contains("first content token"));

        let cross: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("bundle/crosscheck.json")).unwrap(),
        )
        .unwrap();
        let checks = cross["points"][0]["checks"].as_array().unwrap();
        assert!(
            checks.iter().all(|c| c["match"].as_bool() == Some(true)),
            "summary-derived and record-derived arithmetic agree: {checks:?}"
        );

        let point = std::fs::read_to_string(dir.path().join("bundle/point.yaml")).unwrap();
        assert!(point.contains("EXPLICITLY NON-CERTIFYING"));
        assert!(point.contains("scenario_declared: Server"));
        assert!(point.contains("run_id: \"run-1\""));

        let power: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("bundle/system_power.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(power["declared"], false, "power is never synthesized");
        assert!(power["reason"]
            .as_str()
            .unwrap()
            .contains("never synthesized"));

        let compliance =
            std::fs::read_to_string(dir.path().join("bundle/NONCOMPLIANCE.md")).unwrap();
        assert!(compliance.contains("IMPOSSIBLE"));
        assert!(compliance.contains("Accuracy/coverage gating"));
        assert!(compliance.contains("LoadStair/Pareto"));

        let accuracy_marker =
            std::fs::read_to_string(dir.path().join("bundle/accuracy/NOT_PROVIDED")).unwrap();
        assert!(accuracy_marker.contains("never fabricates"));

        // The adapter's own API surface also flows through the CLI.
        assert!(summary.files.iter().any(|f| f == "crosscheck.json"));
    }

    #[test]
    fn partial_only_log_is_not_exportable() {
        let dir = tempfile::tempdir().unwrap();
        let mut rows = run_rows();
        rows[3]["partial"] = json!(true);
        let err = export(dir.path(), &rows, ScenarioKind::Server, None).unwrap_err();
        assert!(err.to_string().contains("partial"), "{err}");
    }

    #[test]
    fn ambiguous_log_is_not_exportable() {
        let dir = tempfile::tempdir().unwrap();
        let mut rows = run_rows();
        let mut second = run_rows();
        for row in &mut second {
            *row.get_mut("run_id").unwrap() = json!("run-2");
        }
        rows.append(&mut second);
        let err = export(dir.path(), &rows, ScenarioKind::Server, None).unwrap_err();
        assert!(
            err.to_string().contains("multiple complete summaries"),
            "{err}"
        );
    }

    #[test]
    fn unsupported_scenario_intents_are_disclosed_never_silent() {
        for (kind, marker) in [
            (ScenarioKind::Offline, "no native Offline burst mode"),
            (ScenarioKind::Loadstair, "not a bench load pattern"),
            (ScenarioKind::Pareto, "not a bench load pattern"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let summary = export(dir.path(), &run_rows(), kind, None).unwrap();
            let disclosure = summary.scenario_disclosure.expect("disclosure");
            assert!(disclosure.contains(marker), "{disclosure}");
            let point = std::fs::read_to_string(dir.path().join("bundle/point.yaml")).unwrap();
            assert!(point.contains("scenario_disclosure"));
        }
    }

    #[test]
    fn declared_power_descriptors_pass_through_with_hash() {
        let dir = tempfile::tempdir().unwrap();
        let power = dir.path().join("power.json");
        std::fs::write(
            &power,
            r#"{"board_power_limit_w": 600, "source": "declared"}"#,
        )
        .unwrap();
        let summary = export(dir.path(), &run_rows(), ScenarioKind::Server, Some(power)).unwrap();
        let file: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("bundle/system_power.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(file["declared"], true);
        assert_eq!(
            file["source_file_hash_sha256"].as_str().unwrap().len(),
            64,
            "sha256 of the descriptor is recorded"
        );
        assert_eq!(file["descriptors"]["board_power_limit_w"], 600);
        let _ = summary;
    }

    #[test]
    fn crosscheck_mismatch_is_flagged() {
        let dir = tempfile::tempdir().unwrap();
        let mut rows = run_rows();
        rows[3]["ttft_s"] = json!({"p50": 9.9, "p90": 0.2});
        let summary = export(dir.path(), &rows, ScenarioKind::Server, None).unwrap();
        assert!(
            summary.warnings.iter().any(|w| w.contains("ttft_p50_ms")),
            "mismatch flagged: {:?}",
            summary.warnings
        );
    }

    #[test]
    fn usage_missing_rows_drop_token_rates_instead_of_undercounting() {
        let dir = tempfile::tempdir().unwrap();
        let mut rows = run_rows();
        rows[1]["usage_missing"] = json!(true);
        rows[1]["completion_tokens"] = json!(0);
        let summary = export(dir.path(), &rows, ScenarioKind::Server, None).unwrap();
        let result: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("bundle/performance/result_summary.json"))
                .unwrap(),
        )
        .unwrap();
        assert!(
            result["result_completed_tokens_per_second"].is_null(),
            "usage-incomplete runs never emit a token rate: {result}"
        );
        let _ = summary;
    }
}
