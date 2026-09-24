<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Telemetry analysis (agent-first)

Load this file with [OUTPUT_SCHEMA.md](../OUTPUT_SCHEMA.md) and
[TELEMETRY.md](../TELEMETRY.md) before inventing metric names. Prefer recipes
in [docs/queries/](../queries/). There is no `correlate` subcommand and no
DuckDB feature in the crate.

## Field dictionary (join surface)

| Kind | Required fields | Notes |
|------|-----------------|-------|
| `run` | `run_id`, `t0_wall`, `tool_version`, `schema_version`, `config` | Optional `sut`, `telemetry_sources[]` with `clock_offset_ms` |
| `stage` | `run_id`, `stage`, `load`, `phase`, `t_start_ns`, `t_end_ns` | `phase` is `warmup` or `measure` |
| `telemetry` | `run_id`, `t_ns`, `src`, `metric`, `value`, `unit`, `mtype`, `scrape_ms` | `labels` map; optional `raw` when scaled |
| `request` | `run_id`, `seq`, `stage`, `warmup`, `t_*_ns`, tokens, latencies | Optional `telemetry_at_done` (sugar) |
| `scrape_error` | `run_id`, `t_ns`, `src`, `error` | Optional `http_status` |
| `summary` | `run_id`, `partial`, row counts, `dropped_telemetry_rows` | Terminal line |

`schema_version` on the run row is `metrum-ai-bench-cli.telemetry.v1`.
`mtype` is one of `counter`, `gauge`, `histogram_bucket`, `summary`, `unknown`.

## Joins

1. Filter stages: `kind = 'stage' AND phase = 'measure'`.
2. Attach telemetry: same `run_id` and `t_ns` in `[t_start_ns, t_end_ns)`.
3. Series key: `(src, metric, sorted labels)`. When summing board power across
   GPUs, group by `labels.gpu` (or UUID) then sum per `t_ns`, or sum label
   slices consistently for the whole stage.
4. Attach requests: same `run_id` and `stage`, with `warmup = false` for
   measure totals. Use `t_sent_ns` / `t_done_ns` only for request-side windows;
   do not treat `telemetry_at_done` as a time series.
5. Knee stage: pick the strategic knee load from the HTML/CSV/stdout summary,
   then restrict telemetry to that `stage` / `load`.

## Counter vs gauge

- **Gauge** (`power`, util, KV %, temperature): use samples as values. For
  means over a stage, time-weight adjacent samples.
- **Counter** (`*_total`, energy accumulators, preemption totals): always take
  Δ = last − first inside the stage window (same series key). Never average
  raw counter values. If the counter resets mid-window, treat the stage as
  invalid for that series.

## Derived metrics (per measured stage)

Let samples of a gauge be `(t_i, v_i)` with `t` in seconds from the epoch
(`t_ns / 1e9`), ordered, restricted to the stage window.

### `power_mean_w` (time-weighted)

\[
\frac{\sum_{i=1}^{n-1} \frac{v_i + v_{i+1}}{2}\,(t_{i+1}-t_i)}{t_n - t_1}
\]

Require `n >= 2`. Sum multi-GPU gauges first if reporting board total.

### `power_p95_w`

Empirical 95th percentile of the per-sample gauge values in the window
(Hyndman-Fan type 7 if matching strategic percentiles). Not time-weighted.

### `energy_j`

1. If an energy counter exists (for example
   `DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION` after unit scale to joules, or
   `nvidia_smi_energy_joules_total`):
   `energy_j = sum_over_gpus(last - first)`.
2. Else trapezoid on power:
   `energy_j = sum_i 0.5 * (v_i + v_{i+1}) * (t_{i+1} - t_i)` with `v` in watts
   and `t` in seconds (yields joules).

Cross-check both when both series exist; they should agree to about 3
significant figures on a stable load.

### `j_per_output_token`

`energy_j / sum(output_tokens)` over successful non-warmup requests in the
same stage. Null when tokens are zero.

### `gpu_util_mean`

Time-weighted mean of a utilization gauge
(`DCGM_FI_DEV_GPU_UTIL`, `all_smi_gpu_utilization`,
`nvidia_smi_utilization_gpu_ratio`, `gpu_gfx_activity`, …). Normalize ratio vs
percent (0-1 vs 0-100) before comparing vendors.

### `sm_active_p50`

Median of `DCGM_FI_PROF_SM_ACTIVE` (or fork equivalent) samples in the window
when the profiling counter is included. Omit when the series is absent.

### `kv_cache_util_mean`

Time-weighted mean of engine KV gauges (`vllm:kv_cache_usage_perc` /
`vllm:gpu_cache_usage_perc`, `sglang:token_usage`,
`trtllm_kv_cache_utilization`, `llamacpp:kv_cache_usage_ratio`).

### `kv_cache_util_at_knee`

Same as `kv_cache_util_mean` restricted to the knee stage/load.

### `preemptions_delta`

Counter Δ of `vllm:num_preemptions_total` or `sglang:num_preemptions_total`
(or TRT-LLM preemption counter) inside the stage window.

## Invariants for re-validation

- `summary.request_rows` equals the number of `kind=request` lines for that
  `run_id` (allowing for a partial file if `partial=true`).
- `summary.telemetry_rows + summary.dropped_telemetry_rows` accounts for
  scrape attempts that produced or dropped samples under backpressure.
- Measure stages have `t_end_ns > t_start_ns`.
- For a configured interval `I` ms and stage duration `D` s, expect roughly
  `D / (I/1000)` scrapes per source when the stage is long compared to `I`.
- Energy from counter Δ and from ∫power should match within ~1% on steady
  closed-loop stages when both series are present and units are correct.

## Recipes

| File | Purpose |
|------|---------|
| [row_counts.sql](../queries/row_counts.sql) | Counts by `kind` |
| [stage_power.sql](../queries/stage_power.sql) | Power samples and crude means per measure stage |
| [energy_crosscheck.sql](../queries/energy_crosscheck.sql) | Counter Δ vs trapezoid power |
| [analyze.py](../queries/analyze.py) | Stdlib-only Python twin of the SQL recipes |

Do not invent metric names that are not in the NDJSON or in
[exporters.md](exporters.md).
