<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Run telemetry (Prometheus scrape)

**Search first, then check the path.** Before a run, confirm the exporter's
current release, its listen port, and the path your installed binary serves
(`curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:9090/metrics`). The
Metrum all-smi fork v0.26.3-metrum.4 serves **`/metrics`**. Its `/metric`
returned HTTP 404 on 2026-10-02, although earlier repo docs gave `/metric`.

Every benchmark binary (`metrum-ai-bench-cli-llm`, `-vlm`, `-asr`,
`-imagegen` and `-strategic`) optionally scrapes Prometheus text or
OpenMetrics exposition during a run and writes tagged NDJSON rows beside
request and stage rows. All five share one writer and one lifecycle
(`TelemetrySession` in `src/telemetry/session.rs`), so the YAML schema, the
row kinds and `telemetry.v1` are the same everywhere. Hardware and engine signals reach the client only
through HTTP GET of a Prometheus `/metrics` endpoint. No
NVML, ROCm, IPMI, or Redfish SDKs live in the binary: a new device is a YAML
source, not a crate dependency.

## The series list is not in the binary

`--telemetry` YAML is loaded at startup. Each source is a URL plus `include`
regexes. The parser keeps every exposition sample whose name matches, and the
startup probe prints `matched_series` from that live response. There is no
compiled catalog of metric names.

Files under [docs/telemetry/examples/](telemetry/examples/) are starting
points. Exporters add and rename series between releases. Before a run, curl
the live page (the Metrum all-smi fork, the serving engine's `/metrics`, and
any other exporter you add) and set `include` from what that page serves.
Record the exporter version in the SUT notes. Client JSONL fields are a
separate schema. They are not the set of series the NDJSON will store.

Legacy `--metrics-url` desugars to one engine source with a small convenience
allowlist (`engine_include_patterns` in `src/telemetry/parser.rs`). Use
`--telemetry` when the page has series outside that allowlist.

## Compared with AIPerf

[AIPerf](https://github.com/ai-dynamo/aiperf) is NVIDIA's replacement for
GenAI-Perf. Its metrics reference is a named client catalog. `--server-metrics`
(on by default) ingests any Prometheus page, not only the inference server's
`/metrics`, and the Metrum all-smi fork page works as a source. AIPerf also
exports raw time-stamped scrapes: `server_metrics_export.parquet` by default
(raw time series with deltas), `server_metrics_export.jsonl` with
`timestamp_ns` per scrape when `--server-metrics-formats` includes `jsonl`,
and `gpu_telemetry_export.jsonl` per record. Its JSON and CSV summaries are
aggregates. See
[server-metrics.md](https://github.com/ai-dynamo/aiperf/blob/v0.13.0/docs/server-metrics/server-metrics.md)
and
[gpu-telemetry.md](https://github.com/ai-dynamo/aiperf/blob/v0.13.0/docs/kubernetes/gpu-telemetry.md).

The difference is narrower. AIPerf derives its power-efficiency family (avg
only) from `--gpu-telemetry` alone, meaning DCGM, pynvml, and amdsmi
([gpu-telemetry-metrics-dataflow.md](https://github.com/ai-dynamo/aiperf/blob/v0.13.0/docs/reference/gpu-telemetry-metrics-dataflow.md)).
Bench CLI writes the series matched by the YAML `include` into one NDJSON next
to the per-request JSONL, so GPU series from any exporter can be correlated
per request.

Bench CLI splits the same job differently. `request.v3` and `summary.v3` are
the fixed client schema. Telemetry is not a second catalog in the binary:
`TelemetryConfig::load` reads YAML at startup, `parse_exposition` keeps samples
whose names match `include`, and the probe prints `matched_series` from the
live page. A count of client JSONL fields, or of names in an example YAML, is
not the set of data points a run stores.

Publish campaign `publish-20261002T162512Z` already did this. Each modality
cell scraped the Metrum all-smi fork (`:9090/metrics`, 500 ms) and the serving
engine (`/metrics`, 1 s) into one `telemetry.ndjson`. LLM, VLM, and ASR used
vLLM; ImageGen used vLLM-Omni. Those cells stored 88,808 all-smi samples and
3,219 engine samples. The strategic remediation sidecar stored 13,678 rows from
the same two endpoints. DCGM and the other exporters under
[docs/telemetry/examples/](telemetry/examples/) are further YAML sources on the
same writer.

To compare a later release, curl the live `/metrics` pages and count series in
that run's NDJSON. Leave the count out of this document. Exporters rename
series between releases.

## Flags

| Flag | Binaries | Role |
|------|----------|------|
| `--ndjson PATH` | all | Tagged NDJSON run log (`run` / `stage` / `request` / `telemetry` / `scrape_error` / `summary`) |
| `--telemetry PATH` | all | YAML listing Prometheus sources (URL, interval, `include` regexes, optional `units`) |
| `--require-telemetry` | all | Abort after consecutive scrape failures (default 3; override with `--require-telemetry-failures`) |
| `--require-telemetry-failures N` | all | Consecutive failures per source before `--require-telemetry` aborts |
| `--metrics-url URL` | strategic | Legacy single engine source; desugars to a YAML-equivalent when `--ndjson` or `--require-telemetry` is set |

Failure policy, the same for every binary:

- **Startup probe.** Before the first request, each source is fetched once.
  A source that refuses the connection, returns 4xx/5xx, or matches zero
  series (without `allow_empty: true`) fails the run with
  `telemetry: a configured source could not be scraped at startup; no
  requests were sent`. This holds with or without `--require-telemetry`.
- **Mid-run.** Without `--require-telemetry`, a failing scrape writes a
  `scrape_error` row and the run continues. With it, N consecutive failures
  on one source stop the run from issuing new requests (or stages), the
  in-flight requests finish, and the binary exits non-zero. Modality
  binaries still write `summary.v3` (with `partial: true`) and close the
  NDJSON with `partial: true` before exiting. Strategic also closes its
  NDJSON with `partial: true` before exiting non-zero.
- A scraper that exits without `--require-telemetry` (for example a panic)
  is a warning and never stops the run.
- If the NDJSON itself cannot be written (for example a full disk), the
  modality binary stops writing rows, still writes `summary.v3`, and then
  exits non-zero.
- `--require-telemetry` without `--telemetry` (or `--metrics-url` on
  strategic) is rejected at startup.
`--telemetry` and telemetry via `--metrics-url` require `--ndjson`. Without
`--ndjson`, scrapes are not persisted. Legacy `--metrics-url` without NDJSON
still feeds the existing strategic correlation path only.

Example:

```bash
# Default smoke: Metrum all-smi fork on loopback /metrics
# Prefer a release binary (x86_64 example):
curl -fsSL -o /tmp/all-smi.tgz \
  https://github.com/chetan-metrum-ai/all-smi/releases/download/v0.26.3-metrum.4/all-smi-linux-x86_64.tar.gz
tar -xzf /tmp/all-smi.tgz -C /tmp && sudo install -m 0755 /tmp/all-smi /usr/local/bin/all-smi
all-smi api --port 9090

metrum-ai-bench-cli-strategic \
  --url http://127.0.0.1:8000/v1/chat/completions \
  --model demo --api-key dummy \
  --sweep 1,2,4 --requests-per-stage 32 \
  --ndjson /tmp/run.ndjson \
  --telemetry docs/telemetry/examples/all-smi.yaml \
  --require-telemetry
```

Modality example (any of llm, vlm, asr, imagegen):

```bash
metrum-ai-bench-cli-llm \
  --url http://127.0.0.1:8000/v1/chat/completions --api-key dummy \
  --model demo --scenario smoke --mode chat --streaming \
  --prompts prompts.jsonl --max-tokens 256 \
  --num-requests 68 --warmup-requests 4 --concurrency 4 \
  --data-log results.jsonl \
  --ndjson run.ndjson \
  --telemetry docs/telemetry/examples/all-smi.yaml \
  --require-telemetry
```

## Modality binaries (llm, vlm, asr, imagegen)

A modality run is one stage. Its NDJSON holds:

- One `run` row first. `config.binary` names the binary; `config` never holds
  API keys.
- One `request` row per `request.v3` record, written as each request
  completes, joined on `seq` and `run_id`. `t_sent_ns` is the record's
  `send_offset_s` in nanoseconds (the run clock starts at the NDJSON epoch,
  after the telemetry probes, so both share one origin), `t_done_ns = t_sent_ns + latency_s`,
  `t_first_ns` is response headers (`first_byte_s`) as on strategic rows,
  and `t_sched_ns` comes from `scheduled_offset_s` in open-loop runs.
  `service_latency_s` is the record's `latency_s`; `latency_s` adds
  `queue_delay_s`, as on strategic rows.
- Up to two `stage` rows, `warmup` and `measure`, written at the end. Each
  window runs from the phase's first send to its last completion. `stage`
  and `load` are the offered load: `--request-rate` in open-loop runs, the
  effective concurrency cap otherwise. Modality binaries do not drain
  warmup before measuring, so at concurrency above 1 the two windows can
  overlap; filter on `phase = 'measure'` as usual.
- `telemetry` and `scrape_error` rows from the scrapers, then one `summary`
  row last.

`summary.v3` gains a `telemetry` block (only with `--ndjson`): the NDJSON file name (no directories),
`sources`, row counts per kind, and `dropped_telemetry_rows`. See
[OUTPUT_SCHEMA.md](OUTPUT_SCHEMA.md).

### Replacing `scripts/live/telemetry_sidecar.py`

The sidecar is deprecated. It stamped wall-clock time, applied no units and
wrote no stage windows. Live cells should pass `--ndjson` and `--telemetry`
to the bench binary instead, with one YAML listing both the all-smi fork
(500 ms) and the engine `/metrics` (1 s) sources that
`scripts/live/widen_cell.sh` starts sidecars for today. The sidecar stays
for binaries built before this change.

## Default source

The checked-in default is the Metrum fork of all-smi:

- Install: https://github.com/chetan-metrum-ai/all-smi
- Listen: `http://127.0.0.1:9090/metrics`. The same path as upstream lablup; `/metric` is not served. The API binds `0.0.0.0` with no bind flag, so firewall port 9090 on shared hosts.
- Example YAML: [docs/telemetry/examples/all-smi.yaml](telemetry/examples/all-smi.yaml)
  (GPU, host memory, aggregate CPU, chassis, energy, and NVLink series;
  per-core CPU rows are left out).
- Per-process rows (`all_smi_process_*`) are off by default. They need
  `all-smi api --processes` plus the commented-out include line in the
  example YAML. Every row carries `pid`, `name`, `user`, and `command` labels,
  and command lines can hold secrets such as `--api-key`. `include` cannot
  drop labels and the NDJSON stores them, so never enable process rows for a
  published run.

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

## Why only `/metrics`

One scrape is one keep-alive HTTP GET plus a streaming text parse. Cost stays
on the order of a few milliseconds of client CPU and a small body (bounded by
`max_body_bytes`, default 16 MiB). The CLI never links vendor SDKs, never
opens BMC sessions, and never speaks Redfish or IPMI JSON: those surfaces
already have Prometheus exporters. Adding Gaudi, ROCm, or a new engine is a
YAML `include` list. The Metrum all-smi fork and the other exporters here use
`/metrics` (Redfish multi-target uses `/redfish?target=...`).

## Why not per-request telemetry as truth

Exporter cadences (often 500 ms to 2 s) do not align with request completion
events. Stamping a single last-seen gauge onto each request breaks
time-weighted means, trapezoid energy, and counter deltas over a stage window.
Durable `kind: telemetry` rows plus stage windows preserve the full series so
offline recipes (DuckDB, `analyze.py`, or an agent) can recompute power, energy,
J/token, and KV at the knee without inventing samples.

## Shadeform e2e prompts

`scripts/e2e/run-shadeform.sh` must extract prompts from Hugging Face
[`metrum-ai/prompt-library`](https://huggingface.co/datasets/metrum-ai/prompt-library)
(default revision `main` = latest; resolved SHA is recorded in
`mix-report.json`). Use `--config sample` and a named `--profile` (default
`rag-medium`). It fails closed on extract errors: do not add a synthetic
prompt fallback. See `docs/PROMPT_LIBRARY.md`.
