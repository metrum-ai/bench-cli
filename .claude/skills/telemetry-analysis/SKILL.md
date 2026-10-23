<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Offline telemetry analysis

Use this skill when asked to analyze a `metrum-ai-bench-cli-strategic` NDJSON
run for power, energy, utilization, KV cache, preemption, or load-sweep knees.
Analysis is offline: do not start a benchmark or contact the serving endpoint.

## Source of truth and boundaries

1. Read [`docs/telemetry/ANALYSIS.md`](../../../docs/telemetry/ANALYSIS.md) and
   [`docs/OUTPUT_SCHEMA.md`](../../../docs/OUTPUT_SCHEMA.md) before deriving
   metrics. The NDJSON log, not request-level `telemetry_at_done`, is the
   source of truth.
2. Confirm there is a `kind: run` row with schema version
   `metrum-ai-bench-cli.telemetry.v1`. Decompress `.zst` or `.gz` inputs to a
   temporary path; never rewrite the source log.
3. Run the tested Rust analysis module on the source log and strategic summary:
   `cargo xtask telemetry-analysis RUN.ndjson --summary SUMMARY.json --json`.
   It emits source choices, stage power/energy checks, engine histogram
   quantiles, knee resolution, and KV-at-knee in JSON. Use
   `--json` without formatting changes when saving machine-readable output.
   For additional derived metrics, prefer DuckDB with `read_ndjson_auto` and
   the SQL recipes in [`docs/telemetry/analysis/`](../../../docs/telemetry/analysis/);
   show joins and formulas so the result is reproducible. There is no Python
   `analyze.py` script.
4. Join telemetry to a `phase = 'measure'` stage by `run_id`, with the half-open
   window `t_ns >= t_start_ns AND t_ns < t_end_ns`. Report the number of
   contributing samples/series (`N`) for every derived result. If `N = 0`, say
   the metric is unavailable; do not interpolate or invent a value.
5. Counters are differenced inside the stage window; gauges are sampled as-is.
   Use time-weighted means for gauges and trapezoidal integration for power
   energy when no valid energy counter is available. Keep all units explicit.

## Power and energy rules

- Treat only draw metrics as power: for example
  `all_smi_gpu_power_consumption_watts`, `DCGM_FI_DEV_POWER_USAGE`, and
  `nvidia_smi_power_draw_watts`. Exclude `power_limit`, `power_cap`, maximum,
  minimum, threshold, chassis, and node-limit series. Do not infer draw from a
  configured limit.
- The all-smi hardware energy counter is named
  `all_smi_gpu_energy_hw_millijoules_total`. Convert raw millijoule deltas to
  joules exactly once (`mJ / 1000`). Ingested rows may already have `unit: J`
  and a `raw` millijoule value; use the scaled `value` in that case and do not
  scale it again. Do not add hardware energy and software-integrated energy
  for the same GPU/window.
- Calculate energy/token only when both the energy delta and output-token
  count are present and nonzero. State the token denominator and GPU/series
  identity.

## Sweep and knee interpretation

Use the recorded strategic points and knee metadata, then verify the underlying
stage rows. A knee must be supported by a measured bend in the points (p95,
throughput, error rate, and load), not merely by the largest tested load. A
flat sweep is `no_bend`; a sweep with fewer than the required points is
`insufficient_points`. Report the candidate load and the evidence for it, or
state that no knee is supported. Do not project a knee beyond tested loads.

## Regression fixtures

The analysis implementation and regression contract live in the tested Rust
module `xtask/src/telemetry_analysis.rs`; checked-in fixtures are at
`test-data/telemetry/analysis/`. Run `cargo test -p xtask telemetry_analysis`
to check quantiles, bucket/reset behavior, source selection, counter scaling,
recorded knee cases, and text/JSON rendering. Re-record the
mock fixtures only with prebuilt release binaries using
`cargo xtask fixtures analyze --bin-dir target/release`; the mock data is not
GPU or performance evidence.
