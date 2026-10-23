// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Regression contracts for the offline telemetry-analysis skill.
//!
//! These tests validate the NDJSON/schema behaviors the skill must preserve;
//! open-ended analysis remains outside the benchmark binary and xtask.

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use std::{fs, path::Path};

    fn root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
    }

    fn fixture(name: &str) -> (Vec<Value>, Value) {
        let directory = root().join("test-data/telemetry/analysis");
        let rows = fs::read_to_string(directory.join(format!("{name}.ndjson")))
            .unwrap()
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let summary = serde_json::from_slice(
            &fs::read(directory.join(format!("{name}.stdout.json"))).unwrap(),
        )
        .unwrap();
        (rows, summary)
    }

    fn power_draw(metric: &str, unit: &str) -> bool {
        metric == "all_smi_gpu_power_consumption_watts"
            || metric == "DCGM_FI_DEV_POWER_USAGE"
            || (metric.contains("power")
                && unit == "W"
                && !["limit", "cap", "max", "min", "threshold", "chassis", "node"]
                    .iter()
                    .any(|marker| metric.to_ascii_lowercase().contains(marker)))
    }

    fn energy_joules(metric: &str, value: f64, already_scaled: bool) -> f64 {
        if metric.to_ascii_lowercase().contains("millijoule") && !already_scaled {
            value * 0.001
        } else {
            value
        }
    }

    #[test]
    fn gpu_power_limits_are_not_power_draw_and_preferred_gauge_is_present() {
        let text =
            fs::read_to_string(root().join("test-data/telemetry/all-smi-fork-h100.prom")).unwrap();
        let typed: Vec<_> = text
            .lines()
            .filter_map(|line| line.strip_prefix("# TYPE "))
            .filter_map(|line| line.split_whitespace().next())
            .collect();
        let draw = typed
            .iter()
            .filter(|metric| power_draw(metric, "W"))
            .copied()
            .collect::<Vec<_>>();
        assert!(draw.contains(&"all_smi_gpu_power_consumption_watts"));
        assert!(typed.iter().any(|metric| metric.contains("power_limit")));
        assert!(!power_draw("all_smi_gpu_power_limit_current_watts", "W"));
        assert!(!draw.is_empty(), "N>0 power sources must be checked");
    }

    #[test]
    fn power_limits_are_excluded_and_raw_or_scaled_millijoules_give_same_energy() {
        for already_scaled in [false, true] {
            let mut power_samples = Vec::new();
            let mut energy_samples = Vec::new();
            for second in 0..=10_u32 {
                let time = u64::from(second) * 1_000_000_000;
                power_samples.push(serde_json::json!({
                    "kind":"telemetry", "t_ns":time,
                    "metric":"all_smi_gpu_power_consumption_watts", "unit":"W", "value":300.0
                }));
                power_samples.push(serde_json::json!({
                    "kind":"telemetry", "t_ns":time,
                    "metric":"all_smi_gpu_power_limit_current_watts", "unit":"W", "value":350.0
                }));
                let raw_millijoules = 1_000_000.0 + f64::from(second) * 300_000.0;
                let mut energy = serde_json::json!({
                    "kind":"telemetry", "t_ns":time,
                    "metric":"all_smi_gpu_energy_hw_millijoules_total", "unit":"J",
                    "value":if already_scaled { raw_millijoules * 0.001 } else { raw_millijoules }
                });
                if already_scaled {
                    energy["raw"] = serde_json::json!(raw_millijoules);
                }
                energy_samples.push(energy);
            }
            let in_window: Vec<_> = power_samples
                .iter()
                .filter(|row| row["t_ns"].as_u64().unwrap() < 10_000_000_000)
                .filter(|row| {
                    power_draw(
                        row["metric"].as_str().unwrap(),
                        row["unit"].as_str().unwrap(),
                    )
                })
                .collect();
            assert_eq!(in_window.len(), 10, "N>0 power samples in [start,end)");
            let mean = in_window
                .iter()
                .map(|row| row["value"].as_f64().unwrap())
                .sum::<f64>()
                / in_window.len() as f64;
            assert_eq!(mean, 300.0);
            let energy_in_window: Vec<_> = energy_samples
                .iter()
                .filter(|row| row["t_ns"].as_u64().unwrap() < 10_000_000_000)
                .collect();
            let counter = |row: &Value| {
                let value = row["value"].as_f64().unwrap();
                energy_joules(
                    row["metric"].as_str().unwrap(),
                    value,
                    row.get("raw").is_some(),
                )
            };
            let delta_joules = counter(energy_in_window.last().unwrap())
                - counter(energy_in_window.first().unwrap());
            assert_eq!(delta_joules, 2_700.0);
            assert_eq!(delta_joules / 1_000.0, 2.7);
        }
        assert_eq!(
            energy_joules("DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION", 4.0, false),
            4.0
        );
    }

    #[test]
    fn recorded_sweep5_has_a_real_knee_and_no_bend_is_explicit() {
        let (rows, sweep) = fixture("sweep5");
        let stages = rows
            .iter()
            .filter(|row| row["kind"] == "stage" && row["phase"] == "measure")
            .count();
        let points = sweep["points"].as_array().unwrap();
        assert!(
            stages > 0 && points.len() >= 5,
            "N>0 stages and sweep points"
        );
        assert_eq!(sweep["knee_detection"]["index"], 2);
        assert_eq!(sweep["points"][2]["load"], 4.0);
        assert!(
            sweep["points"][3]["p95_s"].as_f64().unwrap()
                > sweep["points"][2]["p95_s"].as_f64().unwrap() * 1.2
        );

        let (flat_rows, flat) = fixture("sweep5_no_bend");
        assert!(flat_rows.iter().any(|row| row["kind"] == "stage"));
        let flat_points = flat["points"].as_array().unwrap();
        assert_eq!(flat_points.len(), 5);
        let first = flat_points.first().unwrap()["p95_s"].as_f64().unwrap();
        let last = flat_points.last().unwrap()["p95_s"].as_f64().unwrap();
        assert!(
            last / first < 1.2,
            "flat sweep must not have a 20% p95 bend"
        );
    }

    #[test]
    fn three_point_sweep_is_not_reported_as_a_knee() {
        let (rows, sweep) = fixture("sweep3");
        assert!(rows.iter().any(|row| row["kind"] == "stage"));
        assert_eq!(sweep["points"].as_array().unwrap().len(), 3);
        assert_eq!(sweep["knee_detection"]["reason"], "insufficient_points");
        assert!(sweep["knee"].is_null());
    }
}
