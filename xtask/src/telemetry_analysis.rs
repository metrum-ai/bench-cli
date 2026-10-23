// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Tested offline telemetry-analysis primitives used by the telemetry skill.

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

pub const POWER_PREFERENCE: &[&str] = &[
    "all_smi_gpu_power_consumption_watts",
    "DCGM_FI_DEV_POWER_USAGE",
    "nvidia_smi_power_draw_watts",
    "nv_gpu_power_usage",
    "gpu_power_usage",
    "gpu_package_power",
    "hw_power",
];
pub const ENERGY_PREFERENCE: &[&str] = &[
    "all_smi_gpu_energy_hw_millijoules_total",
    "DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION",
    "nvidia_smi_energy_joules_total",
    "nv_energy_consumption",
    "gpu_energy_consumed",
    "hw_energy",
    "habanalabs_energy",
];
pub const KNEE_MIN_POINTS: usize = 5;
type Labels = BTreeMap<String, String>;
type SeriesKey = (String, Labels);
type Samples = Vec<(u64, f64)>;
pub const DERIVED_FAMILIES: &[(&str, &[&str])] = &[
    (
        "gpu_util_mean",
        &[
            "all_smi_gpu_utilization",
            "DCGM_FI_DEV_GPU_UTIL",
            "nvidia_smi_utilization_gpu_ratio",
            "gpu_gfx_activity",
        ],
    ),
    (
        "sm_active_p50",
        &["all_smi_gpu_sm_active_ratio", "DCGM_FI_PROF_SM_ACTIVE"],
    ),
    (
        "sm_occupancy_p50",
        &["all_smi_gpu_sm_occupancy", "DCGM_FI_PROF_SM_OCCUPANCY"],
    ),
    (
        "tensor_active_p50",
        &[
            "all_smi_gpu_tensor_active_ratio",
            "DCGM_FI_PROF_PIPE_TENSOR_ACTIVE",
        ],
    ),
    (
        "kv_cache_util_mean",
        &[
            "vllm:kv_cache_usage_perc",
            "vllm:gpu_cache_usage_perc",
            "sglang:token_usage",
            "trtllm_kv_cache_utilization",
            "llamacpp:kv_cache_usage_ratio",
        ],
    ),
];

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Quantile {
    pub value: Option<f64>,
    pub reason: Option<String>,
    pub bound: Option<f64>,
}

fn empty_quantile() -> Quantile {
    Quantile {
        value: None,
        reason: None,
        bound: None,
    }
}

pub fn type7(values: &[f64], p: f64) -> Option<f64> {
    if values.is_empty() || !(0.0..=100.0).contains(&p) {
        return None;
    }
    let mut xs = values.to_vec();
    xs.sort_by(f64::total_cmp);
    let h = (xs.len() - 1) as f64 * p / 100.0;
    let lo = h.floor() as usize;
    let hi = h.ceil() as usize;
    if lo == hi {
        return Some(xs[lo]);
    }
    Some(xs[lo] * (hi as f64 - h) + xs[hi] * (h - lo as f64))
}

pub fn histogram_quantile(buckets: &[(f64, f64)], q: f64) -> Quantile {
    if buckets.is_empty() || !(0.0..=1.0).contains(&q) {
        return empty_quantile();
    }
    let mut bins = buckets.to_vec();
    bins.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total = bins.last().map(|b| b.1).unwrap_or(0.0);
    if total <= 0.0 {
        return empty_quantile();
    }
    let rank = q * total;
    let (mut prev_le, mut prev_count) = (None, 0.0);
    for (le, count) in bins {
        if count >= rank {
            if le.is_infinite() {
                return prev_le.map_or_else(empty_quantile, |bound| Quantile {
                    value: None,
                    reason: Some("above_last_bucket".into()),
                    bound: Some(bound),
                });
            }
            if let Some(lower) = prev_le {
                let value = if count == prev_count {
                    le
                } else {
                    lower + (le - lower) * (rank - prev_count) / (count - prev_count)
                };
                return Quantile {
                    value: Some(value),
                    reason: None,
                    bound: None,
                };
            }
            return Quantile {
                value: None,
                reason: Some("below_first_bucket".into()),
                bound: Some(le),
            };
        }
        prev_le = Some(le);
        prev_count = count;
    }
    empty_quantile()
}

pub fn time_weighted_mean(values: &[(u64, f64)]) -> Option<f64> {
    if values.len() < 2 {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by_key(|p| p.0);
    let mut weighted = 0.0;
    let mut elapsed = 0.0;
    for pair in sorted.windows(2) {
        let dt = (pair[1].0 - pair[0].0) as f64 / 1e9;
        if dt > 0.0 {
            weighted += (pair[0].1 + pair[1].1) * 0.5 * dt;
            elapsed += dt;
        }
    }
    (elapsed > 0.0).then_some(weighted / elapsed)
}

pub fn counter_delta(values: &[(u64, f64)]) -> Option<f64> {
    if values.len() < 2 {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by_key(|p| p.0);
    if sorted.windows(2).any(|w| w[1].1 < w[0].1) {
        return None;
    }
    Some(sorted.last()?.1 - sorted.first()?.1)
}

fn string_labels(row: &Value, omit: Option<&str>) -> BTreeMap<String, String> {
    row.get("labels")
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter(|(k, _)| omit != Some(k.as_str()))
                .map(|(k, v)| {
                    (
                        k.clone(),
                        v.as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| v.to_string()),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn source(row: &Value) -> String {
    row.get("src")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}
fn metric(row: &Value) -> &str {
    row.get("metric").and_then(Value::as_str).unwrap_or("")
}
fn time(row: &Value) -> Option<u64> {
    row.get("t_ns").and_then(Value::as_u64)
}
fn value(row: &Value) -> Option<f64> {
    row.get("value").and_then(Value::as_f64)
}

pub fn is_power_draw(row: &Value) -> bool {
    let name = metric(row);
    POWER_PREFERENCE.contains(&name)
        || (name.to_ascii_lowercase().contains("power")
            && row["unit"] == "W"
            && ![
                "limit",
                "cap",
                "max",
                "min",
                "threshold",
                "default",
                "enforced",
                "chassis",
                "node",
                "ipmi",
                "redfish",
            ]
            .iter()
            .any(|part| name.to_ascii_lowercase().contains(part)))
}

pub fn is_energy_counter(row: &Value) -> bool {
    let name = metric(row);
    ENERGY_PREFERENCE.contains(&name)
        || (name.to_ascii_lowercase().contains("energy")
            && row["unit"] == "J"
            && ![
                "limit",
                "cap",
                "max",
                "min",
                "threshold",
                "default",
                "enforced",
                "chassis",
                "node",
                "ipmi",
                "redfish",
            ]
            .iter()
            .any(|part| name.to_ascii_lowercase().contains(part)))
}

pub fn pick_metric(
    rows: &[Value],
    preferred: &[&str],
    predicate: impl Fn(&Value) -> bool,
) -> Option<String> {
    let present: BTreeSet<String> = rows
        .iter()
        .filter(|r| predicate(r))
        .map(|r| metric(r).to_owned())
        .collect();
    preferred
        .iter()
        .find(|name| present.contains(**name))
        .map(|name| (*name).to_owned())
        .or_else(|| present.into_iter().next())
}

pub fn family_metric(rows: &[Value], family: &[&str]) -> Option<String> {
    family
        .iter()
        .find(|candidate| rows.iter().any(|row| metric(row) == **candidate))
        .map(|name| (*name).to_owned())
}

fn family_value(rows: &[Value], name: &str, metric_name: &str) -> Option<f64> {
    let scale = if name == "gpu_util_mean" && metric_name != "nvidia_smi_utilization_gpu_ratio" {
        0.01
    } else {
        1.0
    };
    let mut series: BTreeMap<Labels, Samples> = BTreeMap::new();
    for row in rows.iter().filter(|r| metric(r) == metric_name) {
        if let (Some(t), Some(v)) = (time(row), value(row)) {
            series
                .entry(string_labels(row, None))
                .or_default()
                .push((t, v * scale));
        }
    }
    if name.ends_with("_p50") {
        let samples: Vec<f64> = series
            .values()
            .flat_map(|s| s.iter().map(|(_, v)| *v))
            .collect();
        type7(&samples, 50.0)
    } else {
        let means: Vec<_> = series
            .values()
            .filter_map(|s| time_weighted_mean(s))
            .collect();
        (!means.is_empty()).then(|| means.iter().sum::<f64>() / means.len() as f64)
    }
}

pub fn hollow_source(rows: &[Value]) -> Option<String> {
    if rows
        .iter()
        .any(|r| metric(r) == "all_smi_gpu_hollow_utilization_ratio")
    {
        return Some("all_smi_gpu_hollow_utilization_ratio".into());
    }
    let has_gr = rows
        .iter()
        .any(|r| metric(r) == "DCGM_FI_PROF_GR_ENGINE_ACTIVE");
    let has_sm = rows.iter().any(|r| metric(r) == "DCGM_FI_PROF_SM_ACTIVE");
    (has_gr && has_sm).then(|| "DCGM_FI_PROF_GR_ENGINE_ACTIVE - DCGM_FI_PROF_SM_ACTIVE".into())
}

pub fn preemption_delta(rows: &[Value]) -> Option<f64> {
    for candidate in ["vllm:num_preemptions_total", "sglang:num_preemptions_total"] {
        let matching: Vec<_> = rows.iter().filter(|row| metric(row) == candidate).collect();
        if matching.is_empty() {
            continue;
        }
        let mut series: BTreeMap<SeriesKey, Samples> = BTreeMap::new();
        for row in matching {
            if let (Some(t), Some(v)) = (time(row), value(row)) {
                series
                    .entry((source(row), string_labels(row, None)))
                    .or_default()
                    .push((t, v));
            }
        }
        let deltas: Vec<_> = series
            .values()
            .map(|samples| counter_delta(samples))
            .collect();
        return if !deltas.is_empty() && deltas.iter().all(Option::is_some) {
            Some(deltas.into_iter().flatten().sum())
        } else {
            None
        };
    }
    None
}

pub fn energy_joules(row: &Value) -> Option<f64> {
    let v = value(row)?;
    Some(
        if metric(row).to_ascii_lowercase().contains("millijoule") && row.get("raw").is_none() {
            v * 0.001
        } else {
            v
        },
    )
}

pub fn stage_histogram_quantile(rows: &[Value], base: &str, q: f64) -> Quantile {
    let mut series: BTreeMap<SeriesKey, Samples> = BTreeMap::new();
    for row in rows
        .iter()
        .filter(|r| metric(r) == format!("{base}_bucket"))
    {
        if let (Some(t), Some(v)) = (time(row), value(row)) {
            series
                .entry((source(row), string_labels(row, None)))
                .or_default()
                .push((t, v));
        }
    }
    let mut deltas: BTreeMap<SeriesKey, Vec<(f64, f64)>> = BTreeMap::new();
    let mut reset_sets = BTreeSet::new();
    for ((src, labels), samples) in series {
        let Some(le) = labels.get("le").and_then(|s| {
            if s == "+Inf" {
                Some(f64::INFINITY)
            } else {
                s.parse().ok()
            }
        }) else {
            continue;
        };
        let mut set_labels = labels.clone();
        set_labels.remove("le");
        let set = (src, set_labels);
        match counter_delta(&samples) {
            Some(delta) => {
                deltas.entry(set).or_default().push((le, delta));
            }
            None => {
                reset_sets.insert(set);
            }
        }
    }
    let mut by_le: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    for (set, buckets) in deltas {
        if reset_sets.contains(&set) {
            continue;
        }
        for (le, delta) in buckets {
            let key = if le.is_infinite() {
                "+Inf".to_owned()
            } else {
                format!("{le:.17}")
            };
            let entry = by_le.entry(key).or_insert((le, 0.0));
            entry.1 += delta;
        }
    }
    histogram_quantile(
        &by_le
            .values()
            .map(|(le, count)| (*le, *count))
            .collect::<Vec<_>>(),
        q,
    )
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Knee {
    pub load: Option<Value>,
    pub index: Option<usize>,
    pub reason: Option<String>,
    pub source: Option<String>,
}

pub fn resolve_knee(stdout: Option<&Value>) -> Knee {
    let Some(stdout) = stdout else {
        return Knee {
            load: None,
            index: None,
            reason: Some("no_strategic_stdout".into()),
            source: None,
        };
    };
    let points = stdout
        .get("points")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(det) = stdout.get("knee_detection").and_then(Value::as_object) {
        if det.get("index").is_none_or(Value::is_null) {
            return Knee {
                load: None,
                index: None,
                reason: det.get("reason").and_then(Value::as_str).map(str::to_owned),
                source: Some("knee_detection".into()),
            };
        }
        let i = det.get("index").and_then(Value::as_u64).map(|x| x as usize);
        let Some(i) = i.filter(|i| *i < points.len()) else {
            return Knee {
                load: None,
                index: None,
                reason: Some("knee_index_out_of_range".into()),
                source: Some("knee_detection".into()),
            };
        };
        return Knee {
            load: points[i].get("load").cloned(),
            index: Some(i),
            reason: None,
            source: Some("knee_detection".into()),
        };
    }
    let usable: Vec<_> = points
        .iter()
        .filter(|p| p.get("p95_s").is_some_and(|v| !v.is_null()))
        .collect();
    if usable.len() < KNEE_MIN_POINTS {
        return Knee {
            load: None,
            index: None,
            reason: Some("insufficient_points".into()),
            source: Some("legacy_knee".into()),
        };
    }
    let Some(knee) = stdout.get("knee").filter(|v| !v.is_null()) else {
        return Knee {
            load: None,
            index: None,
            reason: Some("legacy_knee_null".into()),
            source: Some("legacy_knee".into()),
        };
    };
    let load = knee.get("load").cloned();
    let index = points.iter().position(|p| p.get("load") == load.as_ref());
    Knee {
        load,
        index,
        reason: None,
        source: Some("legacy_knee".into()),
    }
}

pub fn analyze_rows(rows: &[Value], stdout: Option<&Value>) -> Value {
    let counts = rows
        .iter()
        .fold(BTreeMap::<String, usize>::new(), |mut c, r| {
            *c.entry(r["kind"].as_str().unwrap_or("?").to_owned())
                .or_default() += 1;
            c
        });
    let stages: Vec<Value> = rows.iter().filter(|s| s["kind"] == "stage" && s["phase"] == "measure").map(|stage| {
        let (Some(start), Some(end)) = (stage["t_start_ns"].as_u64(), stage["t_end_ns"].as_u64()) else { return json!({"stage":stage["stage"],"load":stage["load"],"error":"invalid stage window"}); };
        let window: Vec<Value> = rows.iter().filter(|r| r["kind"] == "telemetry" && r["run_id"] == stage["run_id"] && time(r).is_some_and(|t| start <= t && t < end)).cloned().collect();
        let power = pick_metric(&window, POWER_PREFERENCE, is_power_draw);
        let mut power_by_time = BTreeMap::<u64, f64>::new();
        for row in window.iter().filter(|r| power.as_deref() == Some(metric(r))) {
            if let (Some(t), Some(v)) = (time(row), value(row)) { *power_by_time.entry(t).or_default() += v; }
        }
        let power_samples: Vec<_> = power_by_time.into_iter().collect();
        let power_values: Vec<f64> = power_samples.iter().map(|(_,v)| *v).collect();
        let power_mean = time_weighted_mean(&power_samples);
        let power_p95 = type7(&power_values, 95.0);
        let energy_trap = if power_samples.len() < 2 { None } else { Some(power_samples.windows(2).map(|p| ((p[0].1+p[1].1)*0.5)*((p[1].0-p[0].0) as f64/1e9)).sum::<f64>()) };
        let energy = pick_metric(&window, ENERGY_PREFERENCE, is_energy_counter);
        let mut energy_series: BTreeMap<Labels, Samples> = BTreeMap::new();
        for row in window.iter().filter(|r| energy.as_deref() == Some(metric(r))) {
            if let (Some(t), Some(v)) = (time(row), energy_joules(row)) {
                energy_series.entry(string_labels(row, None)).or_default().push((t, v));
            }
        }
        let energy_delta: Option<f64> = if energy_series.is_empty() { None } else {
            let deltas: Vec<Option<f64>> = energy_series.values().map(|samples| counter_delta(samples)).collect();
            if deltas.iter().all(Option::is_some) { Some(deltas.into_iter().flatten().sum()) } else { None }
        };
        let output_tokens: u64 = rows.iter().filter(|r| r["kind"] == "request" && r["run_id"] == stage["run_id"] && r["stage"] == stage["stage"] && r["success"] == true && r["warmup"] != true).map(|r| r["output_tokens"].as_u64().unwrap_or(0)).sum();
        let energy_for_token = energy_delta.or(energy_trap);
        let joules_per_token = if output_tokens > 0 { energy_for_token.map(|j|j/output_tokens as f64) } else { None };
        let derived_sources: BTreeMap<&str, Option<String>> = DERIVED_FAMILIES.iter().map(|(name, family)| (*name, family_metric(&window, family))).collect();
        let derived_values: BTreeMap<&str, Option<f64>> = DERIVED_FAMILIES.iter().map(|(name,family)| {
            let selected=family_metric(&window,family);
            (*name,selected.as_deref().and_then(|m|family_value(&window,name,m)))
        }).collect();
        let preemption_source = ["vllm:num_preemptions_total", "sglang:num_preemptions_total"].iter().copied().find(|name| window.iter().any(|r|metric(r)==*name));
        let hollow = hollow_source(&window);
        let bases: BTreeSet<String> = window.iter().filter_map(|r| metric(r).strip_suffix("_bucket").filter(|b| b.ends_with("_seconds")).map(str::to_owned)).collect();
        let histograms: BTreeMap<String, Value> = bases.into_iter().map(|b| { let p50=stage_histogram_quantile(&window,&b,0.5); let p95=stage_histogram_quantile(&window,&b,0.95); (b,json!({"p50":p50,"p95":p95})) }).collect();
        json!({"stage":stage["stage"],"load":stage["load"],"power_source":power,"power_mean_w":power_mean,"power_p95_w":power_p95,"energy_source":energy,"energy_counter_j":energy_delta,"energy_trap_j":energy_trap,"output_tokens":output_tokens,"j_per_output_token":joules_per_token,"derived_sources":derived_sources,"derived_values":derived_values,"hollow_source":hollow,"preemption_source":preemption_source,"preemptions_delta":preemption_delta(&window),"engine_histograms":histograms,"telemetry_rows":window.len()})
    }).collect();
    let knee = resolve_knee(stdout);
    let kv_metric = [
        "vllm:kv_cache_usage_perc",
        "vllm:gpu_cache_usage_perc",
        "sglang:token_usage",
        "trtllm_kv_cache_utilization",
        "llamacpp:kv_cache_usage_ratio",
    ];
    let (kv, kv_reason) = if let Some(load) = knee.load.as_ref() {
        if let Some(stage) = rows
            .iter()
            .find(|s| s["kind"] == "stage" && s["phase"] == "measure" && &s["load"] == load)
        {
            if let (Some(start), Some(end)) =
                (stage["t_start_ns"].as_u64(), stage["t_end_ns"].as_u64())
            {
                let window: Vec<_> = rows
                    .iter()
                    .filter(|r| {
                        r["kind"] == "telemetry"
                            && r["run_id"] == stage["run_id"]
                            && time(r).is_some_and(|t| start <= t && t < end)
                    })
                    .cloned()
                    .collect();
                if let Some(name) = family_metric(&window, &kv_metric) {
                    let mut series: BTreeMap<Labels, Samples> = BTreeMap::new();
                    for row in window.iter().filter(|r| metric(r) == name) {
                        if let (Some(t), Some(v)) = (time(row), value(row)) {
                            series
                                .entry(string_labels(row, None))
                                .or_default()
                                .push((t, v));
                        }
                    }
                    let means: Vec<_> = series
                        .values()
                        .filter_map(|s| time_weighted_mean(s))
                        .collect();
                    if means.is_empty() {
                        (None, Some("insufficient_kv_samples"))
                    } else {
                        (Some(means.iter().sum::<f64>() / means.len() as f64), None)
                    }
                } else {
                    (None, Some("no_kv_cache_series"))
                }
            } else {
                (None, Some("invalid_knee_stage_window"))
            }
        } else {
            (None, Some("knee_stage_not_in_ndjson"))
        }
    } else {
        (None, knee.reason.as_deref())
    };
    json!({"row_counts":counts,"stages":stages,"knee":knee,"kv_cache_util_at_knee":kv,"kv_cache_util_at_knee_reason":kv_reason})
}

pub fn render_text(report: &Value) -> String {
    let mut out = String::new();
    out.push_str("row_counts\n");
    if let Some(counts) = report["row_counts"].as_object() {
        for (k, v) in counts {
            out.push_str(&format!("{k}\t{}\n", v));
        }
    }
    out.push_str("\nstage\tload\tpower_source\tenergy_source\ttelemetry_rows\n");
    if let Some(stages) = report["stages"].as_array() {
        for s in stages {
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\n",
                display(&s["stage"]),
                display(&s["load"]),
                display(&s["power_source"]),
                display(&s["energy_source"]),
                display(&s["telemetry_rows"])
            ));
        }
    }
    if let Some(stages) = report["stages"].as_array() {
        for stage in stages {
            if let Some(histograms) = stage["engine_histograms"].as_object() {
                for (name, quantiles) in histograms {
                    out.push_str(&format!(
                        "histogram\t{}\t{}\tp50={}\tp95={}\n",
                        display(&stage["load"]),
                        name,
                        format_quantile(&quantiles["p50"]),
                        format_quantile(&quantiles["p95"])
                    ));
                }
            }
        }
    }
    out.push_str(&format!(
        "\nknee_load\t{}\nkv_cache_util_at_knee\t{}\n",
        display(&report["knee"]["load"]),
        display(&report["kv_cache_util_at_knee"])
    ));
    out
}
fn format_quantile(q: &Value) -> String {
    match q["reason"].as_str() {
        Some("below_first_bucket") => format!("<={}", display(&q["bound"])),
        Some("above_last_bucket") => format!(">{}", display(&q["bound"])),
        _ => display(&q["value"]),
    }
}
fn display(v: &Value) -> String {
    if v.is_null() {
        "null".into()
    } else {
        v.as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| v.to_string())
    }
}

pub fn run(ndjson: &Path, summary: Option<&Path>, json_output: bool) -> Result<()> {
    let text = fs::read_to_string(ndjson).with_context(|| format!("read {}", ndjson.display()))?;
    let rows: Vec<Value> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    let stdout = summary
        .map(fs::read)
        .transpose()?
        .map(|b| serde_json::from_slice(&b))
        .transpose()?;
    let report = analyze_rows(&rows, stdout.as_ref());
    if json_output {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render_text(&report));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf()
    }
    fn fixture(name: &str) -> (Vec<Value>, Value) {
        let dir = root().join("test-data/telemetry/analysis");
        let rows = fs::read_to_string(dir.join(format!("{name}.ndjson")))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let summary =
            serde_json::from_slice(&fs::read(dir.join(format!("{name}.stdout.json"))).unwrap())
                .unwrap();
        (rows, summary)
    }
    fn tele(metric: &str, t: u64, value: f64, labels: Value) -> Value {
        json!({"kind":"telemetry","run_id":"r","src":"s","metric":metric,"t_ns":t,"value":value,"labels":labels})
    }
    #[test]
    fn type7_matches_numpy_linear() {
        assert_eq!(type7(&[4., 1., 3., 2.], 50.), Some(2.5));
        assert!((type7(&[1., 2., 3., 4.], 95.).unwrap() - 3.85).abs() < 1e-12);
        assert!(
            (type7(&(1..=10).map(f64::from).collect::<Vec<_>>(), 90.).unwrap() - 9.1).abs() < 1e-12
        );
        assert_eq!(type7(&[7.], 95.), Some(7.));
        assert_eq!(type7(&[], 50.), None);
    }
    #[test]
    fn histogram_interpolates() {
        let q = histogram_quantile(
            &[(0.1, 10.), (0.5, 30.), (1., 40.), (f64::INFINITY, 40.)],
            0.5,
        );
        assert!((q.value.unwrap() - 0.3).abs() < 1e-12);
        let p95 = histogram_quantile(
            &[(0.1, 10.0), (0.5, 30.0), (1.0, 40.0), (f64::INFINITY, 40.0)],
            0.95,
        );
        assert!((p95.value.unwrap() - 0.9).abs() < 1e-12);
        assert_eq!(q.reason, None);
        assert_eq!(q.bound, None);
    }
    #[test]
    fn histogram_all_mass_below_first_bound() {
        let q = histogram_quantile(
            &[(0.3, 40.), (0.5, 40.), (1., 40.), (f64::INFINITY, 40.)],
            0.95,
        );
        assert_eq!(
            q,
            Quantile {
                value: None,
                reason: Some("below_first_bucket".into()),
                bound: Some(0.3)
            }
        );
    }
    #[test]
    fn histogram_spanning_buckets_and_edges() {
        let b = [(0.1, 10.), (0.5, 30.), (1., 30.), (f64::INFINITY, 40.)];
        assert_eq!(
            histogram_quantile(&b, 0.25).reason.as_deref(),
            Some("below_first_bucket")
        );
        assert!((histogram_quantile(&b, 0.5).value.unwrap() - 0.3).abs() < 1e-12);
        assert_eq!(
            histogram_quantile(&b, 0.95).reason.as_deref(),
            Some("above_last_bucket")
        );
        assert_eq!(
            histogram_quantile(&[(1., 0.), (f64::INFINITY, 0.)], 0.5),
            empty_quantile()
        );
        assert_eq!(histogram_quantile(&[], 0.5), empty_quantile());
        assert_eq!(
            histogram_quantile(&[(f64::INFINITY, 5.)], 0.5),
            empty_quantile()
        );
    }
    #[test]
    fn histogram_all_mass_in_inf() {
        let q = histogram_quantile(&[(0.3, 0.), (1., 0.), (f64::INFINITY, 10.)], 0.5);
        assert_eq!(
            q,
            Quantile {
                value: None,
                reason: Some("above_last_bucket".into()),
                bound: Some(1.)
            }
        );
    }
    #[test]
    fn histogram_empty_and_unbounded_edges_have_no_value() {
        assert_eq!(histogram_quantile(&[], 0.5), empty_quantile());
        assert_eq!(
            histogram_quantile(&[(1.0, 0.0), (f64::INFINITY, 0.0)], 0.5),
            empty_quantile()
        );
        assert_eq!(
            histogram_quantile(&[(f64::INFINITY, 5.0)], 0.5),
            empty_quantile()
        );
    }

    #[test]
    fn stage_histogram_uses_bucket_deltas() {
        let mut rows = Vec::new();
        for (t, cs) in [(0, [100., 100., 100.]), (1_000_000_000, [100., 110., 120.])] {
            for (le, c) in [("0.1", cs[0]), ("0.5", cs[1]), ("+Inf", cs[2])] {
                rows.push(tele(
                    "vllm:time_to_first_token_seconds_bucket",
                    t,
                    c,
                    json!({"le":le,"engine":"0"}),
                ));
            }
        }
        let q = stage_histogram_quantile(&rows, "vllm:time_to_first_token_seconds", 0.5);
        assert!((q.value.unwrap() - 0.5).abs() < 1e-12);
        assert_eq!(rows.len(), 6);
    }
    #[test]
    fn stage_histogram_drops_reset_label_set() {
        let mut rows = Vec::new();
        for (t, e0, e1) in [
            (0, [0., 10., 10.], [50., 60., 60.]),
            (1_000_000_000, [0., 10., 20.], [55., 1., 70.]),
        ] {
            for (engine, cs) in [("0", e0), ("1", e1)] {
                for (le, c) in [("0.1", cs[0]), ("0.5", cs[1]), ("+Inf", cs[2])] {
                    rows.push(tele(
                        "vllm:e2e_request_latency_seconds_bucket",
                        t,
                        c,
                        json!({"le":le,"engine":engine}),
                    ));
                }
            }
        }
        let q = stage_histogram_quantile(&rows, "vllm:e2e_request_latency_seconds", 0.5);
        assert_eq!(
            (q.value, q.reason.as_deref(), q.bound),
            (None, Some("above_last_bucket"), Some(0.5))
        );
        assert_eq!(rows.len(), 12);
    }
    #[test]
    fn engine_histograms_json_and_text() {
        let stage = json!({"kind":"stage","run_id":"r","stage":1,"load":1,"phase":"measure","t_start_ns":0,"t_end_ns":2_000_000_000});
        let mut rows = vec![stage];
        for (t, counts) in [
            (500_000_000, [0.0, 0.0, 0.0]),
            (1_500_000_000, [40.0, 40.0, 40.0]),
        ] {
            for (le, count) in [("0.3", counts[0]), ("0.5", counts[1]), ("+Inf", counts[2])] {
                rows.push(tele(
                    "vllm:request_prefill_time_seconds_bucket",
                    t,
                    count,
                    json!({"le":le,"engine":"0"}),
                ));
            }
        }
        let report = analyze_rows(&rows, None);
        let hist = &report["stages"][0]["engine_histograms"]["vllm:request_prefill_time_seconds"];
        assert_eq!(hist["p50"]["reason"], "below_first_bucket");
        assert_eq!(hist["p50"]["bound"], 0.3);
        let text = render_text(&report);
        assert!(text.contains("p50=<=0.3"));
        assert!(serde_json::from_str::<Value>(&serde_json::to_string(&report).unwrap()).is_ok());
        assert_eq!(rows.len(), 7, "N>0 stage and bucket rows");
    }
    #[test]
    fn one_power_and_energy_source_per_stage_and_dcgm_fallback() {
        let mut rows = vec![
            json!({"kind":"stage","run_id":"r","stage":1,"load":1,"phase":"measure","t_start_ns":0,"t_end_ns":2_000_000_000}),
        ];
        for (t, joules) in [(0, 0.0), (1_000_000_000, 300.0)] {
            rows.push(tele(
                "all_smi_gpu_power_consumption_watts",
                t,
                300.0,
                json!({"gpu":"0"}),
            ));
            rows.push(tele(
                "DCGM_FI_DEV_POWER_USAGE",
                t,
                300.0,
                json!({"gpu":"0"}),
            ));
            rows.push(tele(
                "all_smi_chassis_power_consumption_watts",
                t,
                900.0,
                json!({"gpu":"0"}),
            ));
            rows.push(tele(
                "all_smi_gpu_energy_hw_millijoules_total",
                t,
                joules * 1000.0,
                json!({"gpu":"0"}),
            ));
            rows.push(tele(
                "DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION",
                t,
                joules,
                json!({"gpu":"0"}),
            ));
        }
        for row in rows.iter_mut().filter(|r| {
            r["kind"] == "telemetry"
                && (metric(r).contains("power") || metric(r).contains("energy"))
        }) {
            row["unit"] = json!(if metric(row).contains("power") {
                "W"
            } else {
                "J"
            });
        }
        rows.push(json!({"kind":"request","run_id":"r","stage":1,"success":true,"warmup":false,"output_tokens":1000}));
        let report = analyze_rows(&rows, None);
        let stage = &report["stages"][0];
        assert_eq!(stage["power_source"], "all_smi_gpu_power_consumption_watts");
        assert_eq!(
            stage["energy_source"],
            "all_smi_gpu_energy_hw_millijoules_total"
        );
        assert_eq!(stage["power_mean_w"], 300.0);
        assert_eq!(stage["energy_counter_j"], 300.0);
        assert_eq!(stage["energy_trap_j"], 300.0);
        assert_eq!(stage["output_tokens"], 1000);
        assert_eq!(stage["j_per_output_token"], 0.3);
        let fallback = vec![
            json!({"metric":"DCGM_FI_DEV_POWER_USAGE","unit":"W"}),
            json!({"metric":"node_power_watts","unit":"W"}),
        ];
        assert_eq!(
            pick_metric(&fallback, POWER_PREFERENCE, is_power_draw),
            Some("DCGM_FI_DEV_POWER_USAGE".into())
        );
        assert_eq!(
            pick_metric(&fallback[1..], POWER_PREFERENCE, is_power_draw),
            None
        );
        assert_eq!(rows.len(), 12, "N>0 stage telemetry/request rows");
    }
    #[test]
    fn dcgm_fallback_excludes_node_and_chassis_power() {
        let rows = vec![
            json!({"metric":"ipmi_power_watts","unit":"W"}),
            json!({"metric":"redfish_chassis_power_average_consumed_watts","unit":"W"}),
            json!({"metric":"node_power_watts","unit":"W"}),
            json!({"metric":"DCGM_FI_DEV_POWER_USAGE","unit":"W"}),
        ];
        assert_eq!(
            pick_metric(&rows, POWER_PREFERENCE, is_power_draw),
            Some("DCGM_FI_DEV_POWER_USAGE".into())
        );
        assert_eq!(
            pick_metric(&rows[..3], POWER_PREFERENCE, is_power_draw),
            None
        );
        assert_eq!(rows.len(), 4, "N>0 power source candidates");
    }

    #[test]
    fn all_smi_is_preferred_over_dcgm_for_derived_metrics() {
        let rows = vec![
            tele("all_smi_gpu_utilization", 0, 80.0, json!({})),
            tele("all_smi_gpu_utilization", 1_000_000_000, 80.0, json!({})),
            tele("DCGM_FI_DEV_GPU_UTIL", 0, 10.0, json!({})),
            tele("DCGM_FI_DEV_GPU_UTIL", 1_000_000_000, 10.0, json!({})),
        ];
        assert_eq!(
            family_metric(&rows, DERIVED_FAMILIES[0].1).as_deref(),
            Some("all_smi_gpu_utilization")
        );
        assert_eq!(
            family_value(&rows, "gpu_util_mean", "all_smi_gpu_utilization"),
            Some(0.8)
        );
        assert_eq!(rows.len(), 4, "N>0 competing source samples");
    }

    #[test]
    fn all_smi_names_are_on_recorded_fork_page() {
        let page =
            fs::read_to_string(root().join("scripts/parity/fixtures/all-smi-fork-h100.prom"))
                .unwrap();
        let names = [
            "all_smi_gpu_utilization",
            "all_smi_gpu_sm_active_ratio",
            "all_smi_gpu_sm_occupancy",
            "all_smi_gpu_tensor_active_ratio",
            "all_smi_gpu_hollow_utilization_ratio",
        ];
        for name in names {
            assert!(
                page.lines()
                    .any(|l| l.starts_with(&format!("# TYPE {name} "))),
                "{name}"
            );
        }
        assert_eq!(names.len(), 5);
    }
    #[test]
    fn dcgm_prof_fallback_and_absent_is_none() {
        let rows = [
            tele("DCGM_FI_PROF_SM_ACTIVE", 0, 0.7, json!({"gpu":"0"})),
            tele(
                "DCGM_FI_PROF_SM_ACTIVE",
                1_000_000_000,
                0.7,
                json!({"gpu":"0"}),
            ),
        ];
        assert_eq!(
            type7(&rows.iter().filter_map(value).collect::<Vec<_>>(), 50.),
            Some(0.7)
        );
        let families = vec![
            tele("all_smi_gpu_utilization", 0, 80.0, json!({"gpu":"0"})),
            tele("DCGM_FI_DEV_GPU_UTIL", 0, 50.0, json!({"gpu":"0"})),
            tele("DCGM_FI_PROF_SM_OCCUPANCY", 0, 0.25, json!({"gpu":"0"})),
            tele(
                "DCGM_FI_PROF_PIPE_TENSOR_ACTIVE",
                0,
                0.4,
                json!({"gpu":"0"}),
            ),
            tele("DCGM_FI_PROF_GR_ENGINE_ACTIVE", 0, 0.9, json!({"gpu":"0"})),
        ];
        assert_eq!(
            family_metric(&families, DERIVED_FAMILIES[0].1).as_deref(),
            Some("all_smi_gpu_utilization")
        );
        assert_eq!(
            family_metric(&families, DERIVED_FAMILIES[2].1).as_deref(),
            Some("DCGM_FI_PROF_SM_OCCUPANCY")
        );
        assert_eq!(
            family_value(&families, "sm_occupancy_p50", "DCGM_FI_PROF_SM_OCCUPANCY"),
            Some(0.25)
        );
        assert_eq!(
            family_metric(&families, DERIVED_FAMILIES[3].1).as_deref(),
            Some("DCGM_FI_PROF_PIPE_TENSOR_ACTIVE")
        );
        assert_eq!(
            family_value(
                &families,
                "tensor_active_p50",
                "DCGM_FI_PROF_PIPE_TENSOR_ACTIVE"
            ),
            Some(0.4)
        );
        assert!(
            hollow_source(&families).is_none(),
            "DCGM hollow fallback needs GR and SM samples"
        );
        let mut dcgm_hollow = families.clone();
        dcgm_hollow.push(tele("DCGM_FI_PROF_SM_ACTIVE", 0, 0.7, json!({"gpu":"0"})));
        assert!(hollow_source(&dcgm_hollow)
            .unwrap()
            .contains("DCGM_FI_PROF_GR_ENGINE_ACTIVE"));
        let absent: Vec<Value> = Vec::new();
        assert_eq!(
            pick_metric(
                &absent,
                &["all_smi_gpu_utilization", "DCGM_FI_DEV_GPU_UTIL"],
                |r| metric(r).contains("GPU_UTIL")
            ),
            None
        );
        assert_eq!(absent.len(), 0);
    }
    #[test]
    fn preemption_reset_invalidates_counter() {
        let reset = vec![
            tele("vllm:num_preemptions_total", 0, 5.0, json!({"gpu":"0"})),
            tele("vllm:num_preemptions_total", 1, 7.0, json!({"gpu":"0"})),
            tele("vllm:num_preemptions_total", 2, 1.0, json!({"gpu":"0"})),
        ];
        assert_eq!(preemption_delta(&reset), None);
        let valid = vec![
            tele("vllm:num_preemptions_total", 0, 5.0, json!({"gpu":"0"})),
            tele("vllm:num_preemptions_total", 1, 7.0, json!({"gpu":"0"})),
            tele("vllm:num_preemptions_total", 2, 9.0, json!({"gpu":"0"})),
        ];
        assert_eq!(preemption_delta(&valid), Some(4.0));
        assert_eq!(reset.len(), 3, "N>0 reset samples");
    }
    #[test]
    fn sweep5_and_no_bend_recorded_stage_counts_are_nonzero() {
        for name in ["sweep5", "sweep5_no_bend"] {
            let (rows, summary) = fixture(name);
            let stages = rows
                .iter()
                .filter(|r| r["kind"] == "stage" && r["phase"] == "measure")
                .count();
            assert_eq!(summary["points"].as_array().unwrap().len(), 5);
            assert_eq!(stages, 5);
            assert!(!rows.is_empty(), "N>0 rows for {name}");
        }
    }

    #[test]
    fn recorded_sweep_fixtures_cover_knees_and_n_nonzero() {
        let (mut r, s) = fixture("sweep5");
        assert!(!r.is_empty());
        assert_eq!(resolve_knee(Some(&s)).load, Some(json!(4.)));
        let points = s["points"].as_array().unwrap();
        assert!(points[4]["p95_s"].as_f64().unwrap() / points[0]["p95_s"].as_f64().unwrap() >= 1.2);
        let (_, flat) = fixture("sweep5_no_bend");
        assert_eq!(resolve_knee(Some(&flat)).reason.as_deref(), Some("no_bend"));
        let flat_points = flat["points"].as_array().unwrap();
        assert!(
            flat_points[4]["p95_s"].as_f64().unwrap() / flat_points[0]["p95_s"].as_f64().unwrap()
                < 1.2
        );
        let (_, short) = fixture("sweep3");
        assert_eq!(
            resolve_knee(Some(&short)).reason.as_deref(),
            Some("insufficient_points")
        );
        r.clear();
    }
    #[test]
    fn every_recorded_stage_has_power_and_preemption_scrapes() {
        let mut checked = 0;
        for name in ["sweep5", "sweep5_no_bend", "sweep3"] {
            let (rows, _) = fixture(name);
            for stage in rows
                .iter()
                .filter(|r| r["kind"] == "stage" && r["phase"] == "measure")
            {
                let start = stage["t_start_ns"].as_u64().unwrap();
                let end = stage["t_end_ns"].as_u64().unwrap();
                for m in [
                    "all_smi_gpu_power_consumption_watts",
                    "vllm:num_preemptions_total",
                ] {
                    let n = rows
                        .iter()
                        .filter(|r| {
                            metric(r) == m && time(r).is_some_and(|t| start <= t && t <= end)
                        })
                        .count();
                    assert!(n >= 2, "{name} load {} {m} N={n}", stage["load"]);
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, 26);
    }

    #[test]
    fn knee_index_out_of_range() {
        let s = json!({"points":[{"load":1}],"knee_detection":{"index":9}});
        assert_eq!(
            resolve_knee(Some(&s)).reason.as_deref(),
            Some("knee_index_out_of_range")
        );
    }
    #[test]
    fn legacy_knee_counts_only_points_with_p95() {
        let s = json!({"points":[{"load":1,"p95_s":1.},{"load":2,"p95_s":2.},{"load":3,"p95_s":3.},{"load":4,"p95_s":4.},{"load":5,"p95_s":5.}],"knee":{"load":3}});
        assert_eq!(resolve_knee(Some(&s)).load, Some(json!(3)));
        let short = json!({"points":[{"p95_s":1.},{"p95_s":2.},{"p95_s":null},{"p95_s":4.},{"p95_s":5.}],"knee":{"load":2}});
        assert_eq!(
            resolve_knee(Some(&short)).reason.as_deref(),
            Some("insufficient_points")
        );
    }
    #[test]
    fn knee_detection_uses_supplied_index_when_in_range() {
        let summary = json!({"points":[{"load":1},{"load":2},{"load":4},{"load":8},{"load":16}],"knee_detection":{"index":3}});
        let knee = resolve_knee(Some(&summary));
        assert_eq!(knee.load, Some(json!(8)));
        assert_eq!(knee.source.as_deref(), Some("knee_detection"));
        assert_eq!(summary["points"].as_array().unwrap().len(), 5);
    }

    #[test]
    fn kv_cache_is_resolved_at_knee_and_cli_formats_both_modes() {
        let (rows, summary) = fixture("sweep5");
        let report = analyze_rows(&rows, Some(&summary));
        assert_eq!(report["knee"]["load"], 4.);
        assert_eq!(report["kv_cache_util_at_knee"], 0.25);
        let text = render_text(&report);
        assert!(text.contains("power_source"));
        let json_text = serde_json::to_string(&report).unwrap();
        assert!(serde_json::from_str::<Value>(&json_text).is_ok());
        assert!(!rows.is_empty());
    }

    #[test]
    fn sweep3_current_and_legacy_knees_are_insufficient() {
        let (rows, current) = fixture("sweep3");
        assert_eq!(
            resolve_knee(Some(&current)).reason.as_deref(),
            Some("insufficient_points")
        );
        let mut legacy = current.clone();
        legacy.as_object_mut().unwrap().remove("knee_detection");
        assert_eq!(
            resolve_knee(Some(&legacy)).reason.as_deref(),
            Some("insufficient_points")
        );
        assert!(rows.iter().any(|r| r["kind"] == "stage"));
    }

    #[test]
    fn flat_curve_reason_and_kv_are_null() {
        let (rows, mut summary) = fixture("sweep5");
        summary["knee"] = Value::Null;
        summary["knee_detection"] =
            json!({"index":null,"reason":"flat_curve","points":5,"min_points":5});
        let report = analyze_rows(&rows, Some(&summary));
        assert!(report["kv_cache_util_at_knee"].is_null());
        assert_eq!(report["kv_cache_util_at_knee_reason"], "flat_curve");
        assert!(!rows.is_empty());
    }

    #[test]
    fn recorded_stage_metrics_preserve_fallbacks_and_absent_as_null() {
        for name in ["sweep5", "sweep5_no_bend"] {
            let (rows, summary) = fixture(name);
            let report = analyze_rows(&rows, Some(&summary));
            let stages = report["stages"].as_array().unwrap();
            assert_eq!(stages.len(), 5);
            for stage in stages {
                assert!(stage["power_mean_w"]
                    .as_f64()
                    .is_some_and(|v| (200.0..250.0).contains(&v)));
                assert_eq!(
                    stage["derived_sources"]["sm_active_p50"],
                    "DCGM_FI_PROF_SM_ACTIVE"
                );
                assert_eq!(stage["derived_values"]["sm_active_p50"], 0.55);
                assert!(stage["derived_values"]["gpu_util_mean"]
                    .as_f64()
                    .is_some_and(|v| (0.0..=1.0).contains(&v)));
                assert!(stage["derived_sources"]["sm_occupancy_p50"].is_null());
                assert!(stage["derived_values"]["sm_occupancy_p50"].is_null());
                assert!(stage["derived_sources"]["tensor_active_p50"].is_null());
                assert!(stage["hollow_source"].is_null());
            }
            assert!(!rows.is_empty(), "N>0 recorded rows");
        }
        let empty_report = analyze_rows(&[], None);
        assert!(empty_report["stages"].as_array().unwrap().is_empty());
        assert!(empty_report["kv_cache_util_at_knee"].is_null());
    }

    #[test]
    fn legacy_knee_resolves_kv_at_knee() {
        let (rows, mut summary) = fixture("sweep5");
        summary.as_object_mut().unwrap().remove("knee_detection");
        let report = analyze_rows(&rows, Some(&summary));
        assert_eq!(report["knee"]["source"], "legacy_knee");
        assert_eq!(report["knee"]["load"], 4.0);
        assert_eq!(report["kv_cache_util_at_knee"], 0.25);
        assert!(!rows.is_empty());
    }
}
