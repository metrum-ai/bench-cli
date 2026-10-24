// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Rust implementation of the Bench data-log parity counter.

use anyhow::{bail, Context, Result};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::Path;

const BENCH_SKIP: &[&str] = &[
    "config",
    "environment",
    "sut",
    "per_endpoint",
    "schema_version",
    "errors_by_type",
];
const REQUEST_SKIP: &[&str] = &["seq", "error"];

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Distribution,
    Block,
    Scalar,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quantity {
    pub shape: Shape,
    pub values: usize,
}

#[allow(dead_code)]
impl Shape {
    pub fn label(self) -> &'static str {
        match self {
            Self::Distribution => "distribution",
            Self::Block => "block",
            Self::Scalar => "scalar",
        }
    }
}

#[allow(dead_code)]
pub fn is_num(value: &Value) -> bool {
    value.is_number()
}

fn number(value: &Value) -> Option<f64> {
    value.as_f64().filter(|_| !value.is_boolean())
}

pub fn numeric_leaves(value: &Value) -> usize {
    match value {
        Value::Number(_) => 1,
        Value::Array(values) => values.iter().map(numeric_leaves).sum(),
        Value::Object(values) => values.values().map(numeric_leaves).sum(),
        _ => 0,
    }
}

fn is_distribution(map: &Map<String, Value>) -> bool {
    ["n", "p50", "p99"].iter().all(|key| map.contains_key(*key))
}

fn split_quantities(
    value: &Value,
    path: &str,
    found: &mut BTreeMap<String, (Shape, Vec<Option<f64>>)>,
    empty: &mut BTreeSet<String>,
) -> usize {
    match value {
        Value::Object(map) if is_distribution(map) => {
            if map.get("n").and_then(number) == Some(0.0) {
                empty.insert(path.to_owned());
            }
            found.insert(
                path.to_owned(),
                (Shape::Distribution, distribution_slots(map)),
            );
            0
        }
        Value::Object(map) => map
            .iter()
            .map(|(key, child)| split_quantities(child, &format!("{path}.{key}"), found, empty))
            .sum(),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(index, child)| {
                split_quantities(child, &format!("{path}[{index}]"), found, empty)
            })
            .sum(),
        Value::Number(_) => 1,
        _ => 0,
    }
}

/// Shared quantity collector for data-point schema checks and parity reports.
fn collect_quantities(
    summary: &Value,
    skip: &[&str],
    empty: &mut BTreeSet<String>,
) -> BTreeMap<String, (Shape, Vec<Option<f64>>)> {
    let mut output = BTreeMap::new();
    let Some(object) = summary.as_object() else {
        return output;
    };
    for (key, value) in object {
        if skip.contains(&key.as_str()) {
            continue;
        }
        if let Some(number) = number(value) {
            output.insert(key.clone(), (Shape::Scalar, vec![Some(number)]));
        } else if value.is_object() {
            let leftovers = split_quantities(value, key, &mut output, empty);
            if leftovers > 0 {
                output.insert(key.clone(), (Shape::Block, vec![Some(0.0); leftovers]));
            }
        }
    }
    output
}

/// Quantity map used by schema-derived data-point documentation and parity logs.
#[allow(dead_code)]
pub fn schema_quantities(summary: &Value, skip: &[&str]) -> BTreeMap<String, Quantity> {
    collect_quantities(summary, skip, &mut BTreeSet::new())
        .into_iter()
        .map(|(name, (shape, values))| {
            let number_count = values.iter().filter(|value| value.is_some()).count();
            (
                name,
                Quantity {
                    shape,
                    values: number_count,
                },
            )
        })
        .collect()
}

/// Distinct numeric fields in one record, dotted for nested maps.
#[allow(dead_code)]
pub fn schema_fields(row: &Value, skip: &[&str]) -> BTreeSet<String> {
    fn walk(value: &Value, prefix: &str, top: bool, skip: &[&str], out: &mut BTreeSet<String>) {
        let Some(object) = value.as_object() else {
            return;
        };
        for (key, child) in object {
            if top && skip.contains(&key.as_str()) {
                continue;
            }
            let name = format!("{prefix}{key}");
            if is_num(child)
                || child
                    .as_array()
                    .is_some_and(|items| !items.is_empty() && items.iter().all(is_num))
            {
                out.insert(name);
            } else if child.is_object() {
                walk(child, &format!("{name}."), false, skip, out);
            }
        }
    }
    let mut output = BTreeSet::new();
    walk(row, "", true, skip, &mut output);
    output
}

fn distribution_slots(map: &Map<String, Value>) -> Vec<Option<f64>> {
    let empty = map.get("n").and_then(number) == Some(0.0);
    map.iter()
        .filter_map(|(key, value)| {
            if key == "percentile_method" || key == "p99_unreliable" {
                return None;
            }
            if empty {
                return number(value).map(Some);
            }
            if value.is_null() {
                return Some(None);
            }
            number(value).map(Some)
        })
        .collect()
}

fn duplicate_count(quantities: &BTreeMap<String, Vec<Option<f64>>>) -> usize {
    let mut groups: BTreeMap<String, usize> = BTreeMap::new();
    for values in quantities.values().filter(|values| values.len() >= 2) {
        let key = values
            .iter()
            .map(|value| match value {
                Some(value) => format!("{value:.12}"),
                None => "null".to_owned(),
            })
            .collect::<Vec<_>>()
            .join(",");
        *groups.entry(key).or_default() += 1;
    }
    groups.values().map(|size| size.saturating_sub(1)).sum()
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    }
}

fn read_jsonl(path: &Path) -> Result<Vec<Value>> {
    let reader =
        BufReader::new(File::open(path).with_context(|| format!("open {}", path.display()))?);
    reader
        .lines()
        .filter_map(|line| match line {
            Ok(line) if line.trim().is_empty() => None,
            Ok(line) => Some(serde_json::from_str(&line).map_err(anyhow::Error::from)),
            Err(error) => Some(Err(error.into())),
        })
        .collect()
}

pub fn count_bench(path: &Path, max_ttft_ratio: f64) -> Result<Value> {
    let rows = read_jsonl(path)?;
    let summary = rows
        .iter()
        .rev()
        .find(|row| {
            row["schema_version"]
                .as_str()
                .is_some_and(|v| v.ends_with(".summary.v3"))
        })
        .with_context(|| format!("{}: no summary.v3 record", path.display()))?;
    summary.as_object().context("summary must be an object")?;
    let mut empty = BTreeSet::new();
    let classified = collect_quantities(summary, BENCH_SKIP, &mut empty);
    let mut distributions = 0usize;
    let mut blocks = 0usize;
    let quantities: BTreeMap<String, Vec<Option<f64>>> = classified
        .into_iter()
        .map(|(name, (shape, values))| {
            match shape {
                Shape::Distribution => distributions += 1,
                Shape::Block => blocks += 1,
                Shape::Scalar => {}
            }
            (name, values)
        })
        .collect();
    let measured: Vec<&Value> = rows
        .iter()
        .filter(|row| {
            row["schema_version"]
                .as_str()
                .is_some_and(|v| v.ends_with(".request.v3"))
                && row["phase"] == "measure"
        })
        .collect();
    let mut fields = BTreeSet::new();
    for row in &measured {
        fields.extend(schema_fields(row, REQUEST_SKIP));
    }
    let mut ratios: Vec<f64> = measured
        .iter()
        .filter(|row| row["error"].is_null())
        .filter_map(|row| {
            let ttft = number(&row["ttft_s"])?;
            let e2e = number(&row["latency_s"])?;
            (e2e > 0.0).then_some(ttft / e2e)
        })
        .collect();
    let successes = measured.iter().filter(|row| row["error"].is_null()).count();
    if successes == 0 || ratios.len() < successes {
        bail!(
            "{}: pacing requires TTFT/E2E for each measured success",
            path.display()
        );
    }
    let pacing = median(&mut ratios);
    if pacing > max_ttft_ratio {
        bail!(
            "{}: median TTFT/E2E {pacing:.3} exceeds {max_ttft_ratio}",
            path.display()
        );
    }
    let null_values = quantities
        .values()
        .flatten()
        .filter(|value| value.is_none())
        .count();
    let error_quantities = summary["errors_by_type"].as_object().map_or(0, |errors| {
        errors
            .values()
            .filter(|value| numeric_leaves(value) > 0)
            .count()
    });
    let per_endpoint_quantities = summary["per_endpoint"].as_object().map_or(0, |endpoints| {
        endpoints
            .values()
            .filter_map(Value::as_object)
            .map(|endpoint| {
                endpoint
                    .values()
                    .filter(|value| {
                        number(value).is_some() || (value.is_object() && numeric_leaves(value) > 0)
                    })
                    .count()
            })
            .sum()
    });
    let tool_version = summary["environment"]["package_version"].clone();
    let duplicates = duplicate_count(&quantities);
    Ok(serde_json::json!({
        "tool":"bench", "tool_version":tool_version, "quantities":quantities.len(),
        "values":quantities.values().map(Vec::len).sum::<usize>(),
        "null_values":null_values, "empty_dists":empty.len(),
        "per_request":fields.len(), "duplicates":duplicates, "dists":distributions, "blocks":blocks,
        "measured_requests":measured.len(), "error_quantities":error_quantities,
        "per_endpoint_quantities":per_endpoint_quantities, "median_ttft_over_e2e":pacing
    }))
}

pub fn table(path: &Path) -> Result<String> {
    let mut rows = BTreeMap::<String, BTreeMap<String, Value>>::new();
    for row in read_jsonl(path)? {
        let scenario = row["scenario"].as_str().unwrap_or("?").to_owned();
        let tool = row["tool"]
            .as_str()
            .context("count row missing tool")?
            .to_owned();
        rows.entry(scenario).or_default().insert(tool, row);
    }
    let mut output = String::from(
        "| Scenario   | AIPerf quantities / values / per-request | Bench quantities / values / per-request |\n|------------|------------------------------------------|-----------------------------------------|\n"
    );
    for (scenario, tools) in &rows {
        let format_cell = |tool: &str| -> String {
            tools.get(tool).map_or_else(
                || "n/a".to_owned(),
                |row| {
                    format!(
                        "{} / {} / {}",
                        row["quantities"], row["values"], row["per_request"]
                    )
                },
            )
        };
        output.push_str(&format!(
            "| {scenario:<10} | {:<40} | {:<39} |\n",
            format_cell("aiperf"),
            format_cell("bench")
        ));
    }
    output.push_str("\n| Scenario   | Tool   | dists | empty dists | blocks | duplicates | null values | measured | median TTFT/E2E |\n|------------|--------|-------|-------------|--------|------------|-------------|----------|-----------------|\n");
    for (scenario, tools) in &rows {
        for tool in ["aiperf", "bench"] {
            if let Some(row) = tools.get(tool) {
                let ratio = row["median_ttft_over_e2e"]
                    .as_f64()
                    .map_or_else(|| "n/a".to_owned(), |value| format!("{value:.3}"));
                output.push_str(&format!(
                    "| {scenario:<10} | {tool:<6} | {:>5} | {:>11} | {:>6} | {:>10} | {:>11} | {:>8} | {:>15} |\n",
                    row["dists"], row["empty_dists"], row["blocks"], row["duplicates"],
                    row["null_values"], row["measured_requests"], ratio
                ));
            }
        }
    }
    Ok(output)
}

pub fn count_aiperf(artifact_dir: &Path, max_ttft_ratio: f64) -> Result<Value> {
    let summary_path = artifact_dir.join("profile_export_aiperf.json");
    let summary: Value = serde_json::from_slice(
        &fs::read(&summary_path).with_context(|| format!("read {}", summary_path.display()))?,
    )?;
    let mut quantities: BTreeMap<String, Vec<Option<f64>>> = BTreeMap::new();
    let mut empty = BTreeSet::new();
    let mut distributions = 0usize;
    let mut blocks = 0usize;
    let mut error_quantities = 0usize;
    for (name, block) in summary
        .as_object()
        .context("AIPerf summary must be an object")?
    {
        if [
            "input_config",
            "run_info",
            "warmup_metrics",
            "error_summary",
        ]
        .contains(&name.as_str())
        {
            continue;
        }
        let Some(values) = block
            .as_object()
            .filter(|object| object.contains_key("unit"))
        else {
            continue;
        };
        if name.starts_with("error_") {
            error_quantities += 1;
            continue;
        }
        let count = values.get("count").and_then(number);
        let is_empty = count == Some(0.0);
        let slots: Vec<Option<f64>> = values
            .iter()
            .filter(|(key, _)| key.as_str() != "unit")
            .filter_map(|(_, value)| {
                if is_empty {
                    number(value).map(Some)
                } else if value.is_null() {
                    Some(None)
                } else {
                    number(value).map(Some)
                }
            })
            .collect();
        if slots.iter().all(Option::is_none) {
            continue;
        }
        quantities.insert(name.clone(), slots.clone());
        if is_empty {
            empty.insert(name.clone());
        }
        if slots.len() > 1 {
            if values.contains_key("count") && values.contains_key("p1") {
                distributions += 1;
            } else {
                blocks += 1;
            }
        }
    }
    let records_path = artifact_dir.join("profile_export.jsonl");
    let records = read_jsonl(&records_path).with_context(|| {
        format!(
            "AIPerf records missing or invalid: {}",
            records_path.display()
        )
    })?;
    let measured: Vec<&Value> = records
        .iter()
        .filter(|row| {
            row.pointer("/metadata/benchmark_phase")
                .and_then(Value::as_str)
                == Some("profiling")
        })
        .collect();
    let mut fields = BTreeSet::new();
    let mut ratios = Vec::new();
    for row in &measured {
        if !row["error"].is_null() {
            continue;
        }
        let metrics = &row["metrics"];
        if let Some(map) = metrics.as_object() {
            fields.extend(
                map.iter().filter_map(|(name, metric)| {
                    (!metric["value"].is_null()).then_some(name.clone())
                }),
            );
        }
        if let (Some(ttft), Some(e2e)) = (
            number(&metrics["time_to_first_token"]["value"]),
            number(&metrics["request_latency"]["value"]),
        ) {
            if e2e > 0.0 {
                ratios.push(ttft / e2e);
            }
        }
    }
    let successes = measured.iter().filter(|row| row["error"].is_null()).count();
    if successes == 0 || ratios.len() < successes {
        bail!(
            "{}: pacing requires TTFT/E2E for each AIPerf profiling success",
            artifact_dir.display()
        );
    }
    let pacing = median(&mut ratios);
    if pacing > max_ttft_ratio {
        bail!(
            "{}: median TTFT/E2E {pacing:.3} exceeds {max_ttft_ratio}",
            artifact_dir.display()
        );
    }
    let values = quantities.values().map(Vec::len).sum::<usize>();
    let duplicates = duplicate_count(&quantities);
    let null_values = quantities
        .values()
        .flatten()
        .filter(|value| value.is_none())
        .count();
    Ok(serde_json::json!({
        "tool":"aiperf", "tool_version":summary["aiperf_version"],
        "quantities":quantities.len(), "values":values, "null_values":null_values,
        "empty_dists":empty.len(), "per_request":fields.len(), "duplicates":duplicates,
        "dists":distributions,
        "blocks":blocks, "measured_requests":measured.len(),
        "error_quantities":error_quantities, "median_ttft_over_e2e":pacing
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distribution_slots_keep_nulls_and_empty_distributions_only_count_n() {
        let value: Value =
            serde_json::from_str(r#"{"n":2,"p50":1.0,"p99":null,"std":null}"#).unwrap();
        assert_eq!(
            distribution_slots(value.as_object().unwrap()),
            vec![Some(2.0), Some(1.0), None, None]
        );
        let empty: Value = serde_json::from_str(r#"{"n":0,"p50":null,"p99":null}"#).unwrap();
        assert_eq!(
            distribution_slots(empty.as_object().unwrap()),
            vec![Some(0.0)]
        );
    }

    #[test]
    fn bench_log_counter_requires_and_reports_nonzero_measurements() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("run.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"schema_version":"metrum-ai-bench-cli.request.v3","phase":"measure","ttft_s":0.1,"latency_s":0.5,"output_tokens":4}"#,
                "\n",
                r#"{"schema_version":"metrum-ai-bench-cli.summary.v3","attempted":1,"latency_s":{"n":1,"p50":0.5,"p99":0.5}}"#,
                "\n"
            ),
        ).unwrap();
        let result = count_bench(&path, 0.9).unwrap();
        assert!(result["quantities"].as_u64().unwrap() > 0);
        assert!(result["values"].as_u64().unwrap() > 0);
        assert_eq!(result["measured_requests"], 1);
    }

    #[test]
    fn table_reports_counts_and_null_slot_details() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("counts.jsonl");
        std::fs::write(&path, concat!(
            r#"{"scenario":"plain","tool":"bench","quantities":1,"values":2,"per_request":3,"dists":1,"empty_dists":0,"blocks":0,"duplicates":0,"null_values":1,"measured_requests":4,"median_ttft_over_e2e":0.2}"#,
            "\n"
        )).unwrap();
        let report = table(&path).unwrap();
        assert!(report.contains("1 / 2 / 3"));
        assert!(report.contains("null values"));
        assert!(report.contains("| plain      | bench"));
    }

    #[test]
    fn duplicate_groups_ignore_lone_scalars_and_keep_null_slots() {
        let quantities = BTreeMap::from([
            ("a".to_owned(), vec![Some(1.0), None, Some(2.0)]),
            ("b".to_owned(), vec![Some(1.0), None, Some(2.0)]),
            ("c".to_owned(), vec![Some(1.0), Some(0.0), Some(2.0)]),
            ("lone".to_owned(), vec![None]),
        ]);
        assert_eq!(duplicate_count(&quantities), 1);
    }

    #[test]
    fn nullable_distribution_statistics_keep_value_counts_stable() {
        let numeric: Value = serde_json::from_str(
            r#"{"n":2,"min":1.0,"max":2.0,"avg":1.5,"std":0.25,"mad":0.5,"p50":1.5,"p90":1.9,"p95":1.95,"p99":1.99}"#,
        ).unwrap();
        let null_std: Value = serde_json::from_str(
            r#"{"n":2,"min":1.0,"max":2.0,"avg":1.5,"std":null,"mad":0.5,"p50":1.5,"p90":1.9,"p95":1.95,"p99":1.99}"#,
        ).unwrap();
        let numeric_slots = distribution_slots(numeric.as_object().unwrap());
        let null_slots = distribution_slots(null_std.as_object().unwrap());
        assert_eq!(numeric_slots.len(), null_slots.len());
        assert_eq!(
            numeric_slots.iter().filter(|slot| slot.is_some()).count(),
            10
        );
        assert_eq!(null_slots.iter().filter(|slot| slot.is_some()).count(), 9);
        assert_eq!(null_slots.iter().filter(|slot| slot.is_none()).count(), 1);
    }

    #[test]
    fn aiperf_counter_requires_paced_profile_rows_and_counts_nonzero_metrics() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("profile_export_aiperf.json"),
            r#"{
            "aiperf_version":"0.13.0",
            "request_latency":{"unit":"ms","avg":100.0,"p1":80.0,"count":2},
            "request_throughput":{"unit":"rps","avg":1.0},
            "input_config":{"unit":"n/a","value":1}
        }"#,
        )
        .unwrap();
        std::fs::write(directory.path().join("profile_export.jsonl"), concat!(
            r#"{"metadata":{"benchmark_phase":"profiling"},"metrics":{"time_to_first_token":{"value":20.0},"request_latency":{"value":100.0}}}"#,
            "\n",
            r#"{"metadata":{"benchmark_phase":"profiling"},"metrics":{"time_to_first_token":{"value":20.0},"request_latency":{"value":100.0}}}"#,
            "\n"
        )).unwrap();
        let counts = count_aiperf(directory.path(), 0.9).unwrap();
        assert!(counts["quantities"].as_u64().unwrap() > 0);
        assert!(counts["values"].as_u64().unwrap() > 0);
        assert_eq!(counts["measured_requests"], 2);
        assert_eq!(counts["per_request"], 2);
    }

    #[test]
    fn request_field_walker_counts_nested_metrics_and_array_once() {
        let row = serde_json::json!({"seq":1,"error":null,"itl_s":[0.1,0.2],"modality_metrics":{"prompt_words":3}});
        let fields = schema_fields(&row, REQUEST_SKIP);
        assert_eq!(
            fields,
            BTreeSet::from([
                "itl_s".to_owned(),
                "modality_metrics.prompt_words".to_owned()
            ])
        );
    }
}
