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
5. Knee stage: read `knee_detection.index` from the strategic stdout JSON
   (#190), take `points[index].load`, then restrict telemetry to the measure
   stage with that `load`. When `knee_detection` is absent (older builds), use
   the `knee` object's `load`, and only when 5 or more points have a
   `p95_s` (the #190 rule). An index outside `points` is null with reason
   `knee_index_out_of_range`.

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

Require `n >= 2`. Use one power source per stage, never two exporters for
the same GPU (that doubles the reading): `all_smi_gpu_power_consumption_watts`,
else `DCGM_FI_DEV_POWER_USAGE`, else `nvidia_smi_power_draw_watts` and the
other GPU power gauges. Sum that source's GPUs at each `t_ns` for the board
total. Chassis, node, IPMI and Redfish meters measure wall power; keep them
out of GPU power.

### `power_p95_w`

Empirical 95th percentile of the per-sample gauge values in the window,
Hyndman-Fan type 7 (linear interpolation, the numpy default and the client's
`percentile_method`). Not time-weighted. Every percentile below uses type 7.

### `energy_j`

1. If an energy counter exists, use one source per stage:
   `all_smi_gpu_energy_hw_millijoules_total`, else
   `DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION` (both mJ; scale to joules), else
   `nvidia_smi_energy_joules_total` and the other GPU counters:
   `energy_j = sum_over_gpus(last - first)`.
2. Else trapezoid on power:
   `energy_j = sum_i 0.5 * (v_i + v_{i+1}) * (t_{i+1} - t_i)` with `v` in watts
   and `t` in seconds (yields joules).

Cross-check both when both series exist; they should agree to about 3
significant figures on a stable load.

The mock server's fixture page (`--telemetry-fixture`, used for
`docs/queries/fixtures/`) advances its energy counter by a fixed step per
scrape, unrelated to its power gauge. Counter energy and power × time do not
agree there, so J/token from the mock fixtures is not meaningful; they test
the joins, not the numbers.

### `j_per_output_token`

`energy_j / sum(output_tokens)` over successful non-warmup requests in the
same stage. Null when tokens are zero.

Utilization-style outputs below are ratios in [0, 1]. Scale percent gauges
by 0.01 first. Each metric lists its sources in preference order; use the
first one with samples in the window and report which one it was. A metric
with no source in the window is absent (null), never 0.

The all-smi names are from the Metrum fork (v0.26.3-metrum.4) as recorded in
[`all-smi-fork-h100.prom`](../../scripts/parity/fixtures/all-smi-fork-h100.prom).
The four GPM gauges (`sm_active_ratio`, `sm_occupancy`, `tensor_active_ratio`,
`hollow_utilization_ratio`) need Hopper or later; older GPUs omit them. The
DCGM fallbacks are dcgm-exporter PROF fields and need profiling metrics
enabled.

### `gpu_util_mean`

Time-weighted mean of a utilization gauge, then the mean across GPUs:
`all_smi_gpu_utilization` (percent), `DCGM_FI_DEV_GPU_UTIL` (percent),
`nvidia_smi_utilization_gpu_ratio` (ratio), `gpu_gfx_activity` (AMD, percent).

### `sm_active_p50`

Type 7 median of the samples in the window, pooled across GPUs:
`all_smi_gpu_sm_active_ratio`, else `DCGM_FI_PROF_SM_ACTIVE` (DCGM field
1002, the same counter).

### `sm_occupancy_p50`

Type 7 median of `all_smi_gpu_sm_occupancy` (no `_ratio` suffix; 0-1), else
`DCGM_FI_PROF_SM_OCCUPANCY`.

### `tensor_active_p50`

Type 7 median of `all_smi_gpu_tensor_active_ratio`, else
`DCGM_FI_PROF_PIPE_TENSOR_ACTIVE` (DCGM field 1004). The fork also exports
per-pipe `all_smi_gpu_tensor_{hmma,imma,dfma}_active_ratio`; they are not
summed here.

### `hollow_util_mean`

Time-weighted mean of `all_smi_gpu_hollow_utilization_ratio`, the fork's
graphics-engine-active minus SM-active, clamped at 0. High values mean the
GPU looks busy to `nvidia-smi` while SMs idle. DCGM fallback: pair
`DCGM_FI_PROF_GR_ENGINE_ACTIVE` and `DCGM_FI_PROF_SM_ACTIVE` at the same
`t_ns` and GPU, take `max(0, gr - sm)`, then time-weight.

### `kv_cache_util_mean`

Time-weighted mean of engine KV gauges: `vllm:kv_cache_usage_perc`
(`vllm:gpu_cache_usage_perc` before vLLM v1), `sglang:token_usage`,
`trtllm_kv_cache_utilization`, `llamacpp:kv_cache_usage_ratio`. The vLLM
`_perc` gauges are fractions (1.0 means full) despite the name.

### `kv_cache_util_at_knee`

`kv_cache_util_mean` of the knee stage (Joins, step 5). Null, never 0, when
there is no knee, and the output says why: the `knee_detection.reason` from
the strategic stdout (`insufficient_points` below 5 points, `missing_latency`,
`flat_curve`, `no_bend` when p95 rises less than 20% across the sweep, #232),
or `insufficient_points` for an older output without
`knee_detection` whose sweep has fewer than 5 points with a `p95_s`. Also
null when the knee index is out of range (`knee_index_out_of_range`) or the
knee stage has no KV series or fewer than 2 KV samples.

### `preemptions_delta`

Counter Δ of `vllm:num_preemptions_total` or `sglang:num_preemptions_total`
inside the stage window, summed across series. Null if any series resets.

### Engine histogram percentiles

Engine latency histograms (for example `vllm:time_to_first_token_seconds`)
are cumulative counters. For a stage:

1. Take each `<name>_bucket` series' Δ (last − first) inside the window.
   If any bucket of a label set (engine, model, ...) resets in the window,
   drop that whole label set: a partial histogram skews the quantile.
2. Sum the Δ per `le` across the remaining label sets.
3. Find the first bucket whose cumulative count reaches `q * total`. If it
   has a finite bound on both sides, interpolate linearly between the
   previous bound and this one, as Prometheus `histogram_quantile` does.
4. Do not interpolate at the edges. This departs from Prometheus on purpose:
   - A rank in the first finite bucket has no lower bound. Prometheus
     interpolates from 0, which always gives `q * bound`. (A live vLLM 0.31.0
     run reported `request_prefill_time` p50/p95 = 0.15/0.285 in every stage
     because every value was below the 0.3 s first bound.) Report null with
     reason `below_first_bucket` and `bound` = that first bound. The quantile
     is `<= bound`.
   - A rank in `+Inf` reports null with reason `above_last_bucket` and
     `bound` = the highest finite bound. The quantile is `> bound`.
   - A histogram with only a `+Inf` bucket, or with no observations, is null
     with no reason.

`analyze.py --json` writes `p50`, `p50_reason` and `p50_bound` per
histogram, and the same three keys for `p95`. `p50_reason` and `p50_bound`
are null when `p50` is an interpolated number. The text table prints
`<=0.3` or `>60` in place of a number. To fix an edge result, use a server
with finer buckets near the observed values or read the client-side
percentiles. Do not use the bound as the percentile.

When every observation of a stage falls in a single interior bucket, p50
and p95 only locate that bucket: the interpolated values (for example live
TTFT p50/p95 = 0.03/0.039 s) say the latency is in the 0.02 to 0.04 s
bucket, not where inside it. Read them as that range.

The result is bounded by bucket resolution. It is the server's view of
latency; do not mix it with client-side type 7 percentiles.

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
| [analyze.py](../queries/analyze.py) | Stdlib-only Python: every derived metric above. Pass the strategic stdout JSON as the second argument for `kv_cache_util_at_knee`; `--json` for machine output |
| [fixtures/](../queries/fixtures/) | Recorded mock sweeps (5 and 3 stages) for `test_analyze.py`; `record.sh` re-records them |

Do not invent metric names that are not in the NDJSON or in
[exporters.md](exporters.md).
