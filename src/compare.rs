// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Compare two or more strategic sweep summaries or request CSVs.

use crate::strategic::{summarize_stage, BenchRecord, SweepPoint};
use crate::summary::SloConfig;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct LabeledRun {
    pub label: String,
    pub path: PathBuf,
    pub points: Vec<ComparePoint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComparePoint {
    pub load: f64,
    pub throughput: f64,
    pub p50_s: Option<f64>,
    pub p95_s: Option<f64>,
    pub p99_s: Option<f64>,
    pub goodput: f64,
    pub user_tps_mean: Option<f64>,
    pub users_at_slo: Option<f64>,
    pub cost_per_million_output_tokens: Option<f64>,
    pub error_rate: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompareReport {
    pub schema_version: &'static str,
    pub baseline: String,
    pub runs: Vec<LabeledRunMeta>,
    pub stages: Vec<CompareStage>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LabeledRunMeta {
    pub label: String,
    pub path: String,
    pub stages: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompareStage {
    pub load: f64,
    pub metrics: BTreeMap<String, BTreeMap<String, Value>>,
}

/// Load a strategic stdout JSON (with `points`) or a per-request CSV.
pub fn load_run(path: &Path, label: &str) -> Result<LabeledRun> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let points = match ext.as_str() {
        "csv" => points_from_csv(path)?,
        "json" => points_from_json(path)?,
        _ => {
            // Try JSON first, then CSV.
            match points_from_json(path) {
                Ok(points) => points,
                Err(json_err) => points_from_csv(path).with_context(|| {
                    format!(
                        "failed to parse {} as strategic JSON ({json_err}) or request CSV",
                        path.display()
                    )
                })?,
            }
        }
    };
    Ok(LabeledRun {
        label: label.to_string(),
        path: path.to_path_buf(),
        points,
    })
}

fn points_from_json(path: &Path) -> Result<Vec<ComparePoint>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let value: Value = serde_json::from_str(&raw)
        .with_context(|| format!("invalid JSON in {}", path.display()))?;
    let array = value
        .get("points")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("{}: missing points[] array", path.display()))?;
    let mut out = Vec::with_capacity(array.len());
    for point in array {
        out.push(compare_point_from_value(point)?);
    }
    if out.is_empty() {
        bail!("{}: points[] is empty", path.display());
    }
    Ok(out)
}

fn compare_point_from_value(point: &Value) -> Result<ComparePoint> {
    let load = point
        .get("load")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| anyhow::anyhow!("point missing load"))?;
    let throughput = point
        .get("throughput")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let p50_s = point.get("p50_s").and_then(|v| v.as_f64());
    let p95_s = point.get("p95_s").and_then(|v| v.as_f64());
    let p99_s = point.get("p99_s").and_then(|v| v.as_f64());
    let goodput = point.get("goodput").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let user_tps_mean = point
        .pointer("/user_tps/avg")
        .and_then(|v| v.as_f64())
        .or_else(|| point.pointer("/user_tps/mean").and_then(|v| v.as_f64()))
        .or_else(|| point.get("user_tps_mean").and_then(|v| v.as_f64()));
    let users_at_slo = point.get("users_at_slo").and_then(|v| v.as_f64());
    let cost_per_million_output_tokens = point
        .get("cost_per_million_output_tokens")
        .and_then(|v| v.as_f64());
    let error_rate = point.get("error_rate").and_then(|v| v.as_f64());
    Ok(ComparePoint {
        load,
        throughput,
        p50_s,
        p95_s,
        p99_s,
        goodput,
        user_tps_mean,
        users_at_slo,
        cost_per_million_output_tokens,
        error_rate,
    })
}

fn points_from_csv(path: &Path) -> Result<Vec<ComparePoint>> {
    let mut reader = csv::Reader::from_path(path)
        .with_context(|| format!("failed to open CSV {}", path.display()))?;
    let records: Vec<BenchRecord> = reader
        .deserialize()
        .collect::<Result<_, _>>()
        .with_context(|| format!("failed to deserialize strategic CSV {}", path.display()))?;
    if records.is_empty() {
        bail!("{}: CSV has no request rows", path.display());
    }
    let mut by_stage: BTreeMap<u64, Vec<BenchRecord>> = BTreeMap::new();
    for record in records {
        let key = (record.stage * 1000.0).round() as u64;
        by_stage.entry(key).or_default().push(record);
    }
    let slos = SloConfig::default();
    let mut points = Vec::new();
    for group in by_stage.values() {
        let load = group.first().map(|r| r.stage).unwrap_or(0.0);
        let measured: Vec<&BenchRecord> = group.iter().filter(|r| !r.warmup).collect();
        if measured.is_empty() {
            continue;
        }
        let window = stage_window_seconds(&measured);
        let sweep = summarize_stage(load, group, window, &slos, None);
        points.push(compare_point_from_sweep(&sweep));
    }
    if points.is_empty() {
        bail!(
            "{}: no measurable stages after excluding warmup",
            path.display()
        );
    }
    Ok(points)
}

fn stage_window_seconds(records: &[&BenchRecord]) -> f64 {
    let mut min_ns = u128::MAX;
    let mut max_end = 0u128;
    for record in records {
        min_ns = min_ns.min(record.sent_unix_ns);
        let end = record.sent_unix_ns + ((record.service_latency_s.max(0.0) * 1e9) as u128);
        max_end = max_end.max(end);
    }
    if max_end <= min_ns {
        return f64::EPSILON;
    }
    (max_end - min_ns) as f64 / 1e9
}

fn compare_point_from_sweep(point: &SweepPoint) -> ComparePoint {
    ComparePoint {
        load: point.load,
        throughput: point.throughput,
        p50_s: point.p50_s,
        p95_s: point.p95_s,
        p99_s: point.p99_s,
        goodput: point.goodput,
        user_tps_mean: point.user_tps.avg,
        users_at_slo: point.users_at_slo,
        cost_per_million_output_tokens: point.cost_per_million_output_tokens,
        error_rate: point.error_rate,
    }
}

/// Build a labeled delta table. The first run is the baseline.
pub fn compare_runs(runs: &[LabeledRun]) -> Result<CompareReport> {
    if runs.len() < 2 {
        bail!("compare requires at least two labeled runs");
    }
    let baseline = runs[0].label.clone();
    let mut loads: Vec<f64> = runs
        .iter()
        .flat_map(|run| run.points.iter().map(|p| p.load))
        .collect();
    loads.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    loads.dedup_by(|a, b| (*a - *b).abs() < 1e-9);

    let metric_names = [
        "throughput",
        "p50_s",
        "p95_s",
        "p99_s",
        "goodput",
        "user_tps_mean",
        "users_at_slo",
        "cost_per_million_output_tokens",
        "error_rate",
    ];

    let mut stages = Vec::new();
    for load in loads {
        let mut metrics: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
        for name in metric_names {
            let mut row = BTreeMap::new();
            let mut baseline_val: Option<f64> = None;
            for (index, run) in runs.iter().enumerate() {
                let value = run
                    .points
                    .iter()
                    .find(|p| (p.load - load).abs() < 1e-9)
                    .and_then(|p| metric_value(p, name));
                if index == 0 {
                    baseline_val = value;
                }
                match value {
                    Some(v) => {
                        row.insert(run.label.clone(), json!(v));
                    }
                    None => {
                        row.insert(run.label.clone(), Value::Null);
                    }
                }
                if index > 0 {
                    if let (Some(base), Some(v)) = (baseline_val, value) {
                        let delta = v - base;
                        let pct = if base.abs() > f64::EPSILON {
                            Some(delta / base * 100.0)
                        } else {
                            None
                        };
                        row.insert(format!("delta_{}_vs_{}", run.label, baseline), json!(delta));
                        row.insert(
                            format!("pct_{}_vs_{}", run.label, baseline),
                            match pct {
                                Some(p) => json!(p),
                                None => Value::Null,
                            },
                        );
                    }
                }
            }
            metrics.insert(name.to_string(), row);
        }
        stages.push(CompareStage { load, metrics });
    }

    Ok(CompareReport {
        schema_version: "metrum-ai-bench-cli.compare.v1",
        baseline,
        runs: runs
            .iter()
            .map(|run| LabeledRunMeta {
                label: run.label.clone(),
                path: run.path.display().to_string(),
                stages: run.points.len(),
            })
            .collect(),
        stages,
    })
}

fn metric_value(point: &ComparePoint, name: &str) -> Option<f64> {
    match name {
        "throughput" => Some(point.throughput),
        "p50_s" => point.p50_s,
        "p95_s" => point.p95_s,
        "p99_s" => point.p99_s,
        "goodput" => Some(point.goodput),
        "user_tps_mean" => point.user_tps_mean,
        "users_at_slo" => point.users_at_slo,
        "cost_per_million_output_tokens" => point.cost_per_million_output_tokens,
        "error_rate" => point.error_rate,
        _ => None,
    }
}

/// Resolve labels for input paths (`--labels` or file stems).
pub fn resolve_labels(paths: &[PathBuf], labels: Option<Vec<String>>) -> Result<Vec<String>> {
    if let Some(labels) = labels {
        if labels.len() != paths.len() {
            bail!(
                "--labels count ({}) must match input file count ({})",
                labels.len(),
                paths.len()
            );
        }
        return Ok(labels);
    }
    Ok(paths
        .iter()
        .map(|path| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("run")
                .to_string()
        })
        .collect())
}

/// Markdown delta table (readable without reading source).
pub fn format_markdown(report: &CompareReport) -> String {
    let mut out = String::new();
    out.push_str("# Strategic compare\n\n");
    out.push_str(&format!("Baseline: **{}**\n\n", report.baseline));
    out.push_str("| Load | Metric |");
    for run in &report.runs {
        out.push_str(&format!(" {} |", run.label));
    }
    for run in report.runs.iter().skip(1) {
        out.push_str(&format!(" delta {} vs {} |", run.label, report.baseline));
        out.push_str(&format!(" pct {} vs {} |", run.label, report.baseline));
    }
    out.push('\n');
    out.push_str("| --- | --- |");
    for _ in &report.runs {
        out.push_str(" --- |");
    }
    for _ in report.runs.iter().skip(1) {
        out.push_str(" --- | --- |");
    }
    out.push('\n');

    for stage in &report.stages {
        for (metric, row) in &stage.metrics {
            out.push_str(&format!("| {:.4} | {} |", stage.load, metric));
            for run in &report.runs {
                out.push_str(&format!(" {} |", fmt_cell(row.get(&run.label))));
            }
            for run in report.runs.iter().skip(1) {
                let dkey = format!("delta_{}_vs_{}", run.label, report.baseline);
                let pkey = format!("pct_{}_vs_{}", run.label, report.baseline);
                out.push_str(&format!(" {} |", fmt_cell(row.get(&dkey))));
                out.push_str(&format!(" {} |", fmt_pct(row.get(&pkey))));
            }
            out.push('\n');
        }
    }
    out
}

fn fmt_cell(value: Option<&Value>) -> String {
    match value {
        Some(Value::Null) | None => "-".into(),
        Some(Value::Number(n)) => {
            if let Some(f) = n.as_f64() {
                format!("{f:.4}")
            } else {
                n.to_string()
            }
        }
        Some(other) => other.to_string(),
    }
}

fn fmt_pct(value: Option<&Value>) -> String {
    match value.and_then(|v| v.as_f64()) {
        Some(f) => format!("{f:.2}%"),
        None => "-".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_summary(path: &Path, load_throughputs: &[(f64, f64, f64)]) {
        let points: Vec<Value> = load_throughputs
            .iter()
            .map(|(load, thr, p95)| {
                json!({
                    "load": load,
                    "throughput": thr,
                    "p50_s": p95 * 0.5,
                    "p95_s": p95,
                    "p99_s": p95 * 1.2,
                    "goodput": thr,
                    "user_tps": {"avg": 20.0},
                    "users_at_slo": null,
                    "cost_per_million_output_tokens": null,
                    "error_rate": 0.0
                })
            })
            .collect();
        let doc = json!({"points": points});
        std::fs::write(path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
    }

    #[test]
    fn compares_two_json_summaries() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("h100.json");
        let b = dir.path().join("pro6000.json");
        sample_summary(&a, &[(1.0, 10.0, 0.2), (2.0, 18.0, 0.3)]);
        sample_summary(&b, &[(1.0, 12.0, 0.18), (2.0, 20.0, 0.28)]);
        let runs = vec![
            load_run(&a, "H100").unwrap(),
            load_run(&b, "Pro6000").unwrap(),
        ];
        let report = compare_runs(&runs).unwrap();
        assert_eq!(report.baseline, "H100");
        assert_eq!(report.stages.len(), 2);
        let thr = &report.stages[0].metrics["throughput"];
        assert_eq!(thr["H100"], json!(10.0));
        assert_eq!(thr["Pro6000"], json!(12.0));
        assert_eq!(thr["delta_Pro6000_vs_H100"], json!(2.0));
        let md = format_markdown(&report);
        assert!(md.contains("throughput"));
        assert!(md.contains("Pro6000"));
    }

    #[test]
    fn label_count_must_match() {
        let err = resolve_labels(
            &[PathBuf::from("a.json"), PathBuf::from("b.json")],
            Some(vec!["only".into()]),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("--labels"));
    }

    #[test]
    fn csv_round_trip_compare() {
        let dir = tempfile::tempdir().unwrap();
        let csv_path = dir.path().join("requests.csv");
        let mut w = csv::Writer::from_path(&csv_path).unwrap();
        let base_ns = 1_000_000_000u128;
        for (stage, i) in [(1.0_f64, 0u64), (1.0, 1), (2.0, 0), (2.0, 1)] {
            let record = BenchRecord {
                seq: i,
                stage,
                endpoint: "http://x".into(),
                scheduled_unix_ns: base_ns + (i as u128 * 100_000_000),
                sent_unix_ns: base_ns + (i as u128 * 100_000_000),
                latency_s: 0.1,
                queue_delay_s: 0.0,
                service_latency_s: 0.1,
                first_byte_s: Some(0.01),
                connect_s: Some(0.0),
                ttft_s: Some(0.02),
                prefill_s: Some(0.02),
                decode_s: Some(0.08),
                decode_tok_s: Some(200.0),
                itl_s: vec![],
                in_flight_at_send: Some(1),
                success: true,
                valid: Some(true),
                input_tokens: 8,
                output_tokens: 16,
                session_id: None,
                turn: None,
                error: None,
                warmup: false,
            };
            w.serialize(&record).unwrap();
        }
        w.flush().unwrap();
        let run = load_run(&csv_path, "csv").unwrap();
        assert_eq!(run.points.len(), 2);
    }
}
