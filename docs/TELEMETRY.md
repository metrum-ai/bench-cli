<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Strategic telemetry (Prometheus scrape)

`metrum-ai-bench-cli-strategic` optionally scrapes Prometheus text or
OpenMetrics exposition during a sweep and writes tagged NDJSON rows beside
request and stage rows. Hardware and engine signals reach the client only
through HTTP GET of `/metrics` (or `/metric` on the Metrum all-smi fork). No
NVML, ROCm, IPMI, or Redfish SDKs live in the binary: a new device is a YAML
source, not a crate dependency.

## Flags

| Flag | Role |
|------|------|
| `--ndjson PATH` | Tagged NDJSON run log (`run` / `stage` / `request` / `telemetry` / `scrape_error` / `summary`) |
| `--telemetry PATH` | YAML listing Prometheus sources (URL, interval, `include` regexes, optional `units`) |
| `--require-telemetry` | Abort after consecutive scrape failures (default 3; override with `--require-telemetry-failures`) |
| `--metrics-url URL` | Legacy single engine source; desugars to a YAML-equivalent when `--ndjson` or `--require-telemetry` is set |

`--telemetry` and telemetry via `--metrics-url` require `--ndjson`. Without
`--ndjson`, scrapes are not persisted. Legacy `--metrics-url` without NDJSON
still feeds the existing strategic correlation path only.

Example:

```bash
# Default smoke: Metrum all-smi fork on loopback /metric
cargo install --git https://github.com/chetan-metrum-ai/all-smi --locked
all-smi api --port 9090

metrum-ai-bench-cli-strategic \
  --url http://127.0.0.1:8000/v1/chat/completions \
  --model demo --api-key dummy \
  --sweep 1,2,4 --requests-per-stage 32 \
  --ndjson /tmp/run.ndjson \
  --telemetry docs/telemetry/examples/all-smi.yaml \
  --require-telemetry
```

## Default source

The checked-in default is the Metrum fork of all-smi:

- Install: https://github.com/chetan-metrum-ai/all-smi
- Listen: `http://127.0.0.1:9090/metric` (fork path; upstream lablup uses `/metrics`)
- Example YAML: [docs/telemetry/examples/all-smi.yaml](telemetry/examples/all-smi.yaml)

Bind exporters to `127.0.0.1` on the serving host when possible. Example YAMLs
for DCGM, ROCm, engines, and BMC exporters live under
[docs/telemetry/examples/](telemetry/examples/). Install one-liners:
[docs/telemetry/exporters.md](telemetry/exporters.md).

## Units at ingest

Each telemetry sample is stored with a `unit` string. Sources may declare
`units:` maps in YAML:

```yaml
units:
  DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION:
    scale: 0.001
    unit: J
  habanalabs_power_mW:
    scale: 0.001
    unit: W
```

When `scale != 1`, the scaled value is written to `value` and the pre-scale
sample is kept in optional `raw`. Without an entry, the scraper guesses a unit
from the metric name (`W`, `C`, `J`, `B`, or `1`). Prefer explicit `units` for
energy counters and milliwatt gauges.

## Join model

All timestamps are nanoseconds from a single run-wide monotonic epoch captured
at start (`run.t0_wall` is the ISO 8601 UTC wall anchor for that Instant).

| Kind | Join keys |
|------|-----------|
| `run` | `run_id` |
| `stage` | `run_id`, window `[t_start_ns, t_end_ns)`, `stage`, `phase` |
| `telemetry` | `run_id`, `t_ns`, `src`, `metric`, `labels` |
| `request` | `run_id`, `t_sched_ns` / `t_sent_ns` / `t_done_ns`, `stage` |
| `summary` | `run_id` (terminal counts and `dropped_telemetry_rows`) |

Attach telemetry to a measured stage with
`t_ns >= t_start_ns AND t_ns < t_end_ns AND phase = 'measure'`. Series identity
is `(src, metric, labels)`. Counters are always differenced inside a window;
gauges are sampled as-is and time-weighted when integrating.

Optional `request.telemetry_at_done` is last-seen sugar for debugging. It is
not the source of truth for power, energy, or KV math. Schema details:
[OUTPUT_SCHEMA.md](OUTPUT_SCHEMA.md). Analysis formulas and agent joins:
[telemetry/ANALYSIS.md](telemetry/ANALYSIS.md). Recipes:
[queries/](queries/).

## Worked DuckDB queries

Optional DuckDB CLI (`read_ndjson_auto`). Equivalent stdlib Python:
`docs/queries/analyze.py`.

Row counts by kind:

```sql
SELECT kind, count(*) AS n
FROM read_ndjson_auto('/tmp/run.ndjson')
GROUP BY 1
ORDER BY 1;
```

Power samples in measure windows (see also `docs/queries/stage_power.sql`):

```sql
WITH stages AS (
  SELECT * FROM read_ndjson_auto('/tmp/run.ndjson')
  WHERE kind = 'stage' AND phase = 'measure'
),
power AS (
  SELECT * FROM read_ndjson_auto('/tmp/run.ndjson')
  WHERE kind = 'telemetry'
    AND metric IN (
      'all_smi_gpu_power_consumption_watts',
      'DCGM_FI_DEV_POWER_USAGE',
      'nvidia_smi_power_draw_watts'
    )
)
SELECT s.stage, s.load, count(p.*) AS power_samples
FROM stages s
LEFT JOIN power p
  ON p.run_id = s.run_id
 AND p.t_ns >= s.t_start_ns
 AND p.t_ns < s.t_end_ns
GROUP BY 1, 2
ORDER BY 1;
```

Energy cross-check (counter Δ vs trapezoid ∫ power):

```sql
-- Prefer docs/queries/energy_crosscheck.sql for the full recipe.
SELECT 'see docs/queries/energy_crosscheck.sql' AS recipe;
```

## Why only `/metrics` (and `/metric` for all-smi)

One scrape is one keep-alive HTTP GET plus a streaming text parse. Cost stays
on the order of a few milliseconds of client CPU and a small body (bounded by
`max_body_bytes`, default 16 MiB). The CLI never links vendor SDKs, never
opens BMC sessions, and never speaks Redfish or IPMI JSON: those surfaces
already have Prometheus exporters. Adding Gaudi, ROCm, or a new engine is a
YAML `include` list. The Metrum all-smi fork exposes exposition at `/metric`
while keeping the same Prometheus text format; other exporters use `/metrics`
(Redfish multi-target uses `/redfish?target=...`).

## Why not per-request telemetry as truth

Exporter cadences (often 500 ms to 2 s) do not align with request completion
events. Stamping a single last-seen gauge onto each request breaks
time-weighted means, trapezoid energy, and counter deltas over a stage window.
Durable `kind: telemetry` rows plus stage windows preserve the full series so
offline recipes (DuckDB, `analyze.py`, or an agent) can recompute power, energy,
J/token, and KV at the knee without inventing samples.
