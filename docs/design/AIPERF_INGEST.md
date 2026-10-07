<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Design note: ingest AIPerf exports

- Issue: [#205](https://github.com/metrum-ai/bench-cli/issues/205) (D6, Track D
  [#188](https://github.com/metrum-ai/bench-cli/issues/188), epic
  [#184](https://github.com/metrum-ai/bench-cli/issues/184))
- Status: proposed. This is a design note only; no production code ships with it.
- Date: 2026-10-05
- Author: Metrum AI

## Question

Should Metrum AI Bench CLI read an NVIDIA AIPerf `profile_export.jsonl` and
add cost, SUT provenance, and telemetry joins on top?

## Recommendation

**Yes, as a narrow importer, and only after Track B lands.**

1. Add `metrum-ai-bench-cli import aiperf <artifact-dir>` to the dispatcher.
   It reads the per-request `profile_export.jsonl` and writes a normal
   `--data-log` JSONL: `request.v3` lines followed by a `summary.v3` line.
2. It recomputes the summary from the imported records with the existing
   `RunSummary::from_records_with_options` (`src/summary.rs`). It never copies
   AIPerf's aggregate numbers into `summary.v3`. `profile_export_aiperf.json`
   is read only for run metadata and as a cross-check.
3. It accepts `--sut`, `--require-sut`, and `--price-per-hour` with the same
   meaning as a live run. A new additive `import` block marks every imported
   summary, so an imported number is never mistaken for a Bench measurement.
4. Phase 1 is LLM chat and completions, streaming and non-streaming, for AIPerf
   0.11 to 0.13. It gates on the observed pair of `aiperf_version` and
   `schema_version`, not on the schema number alone: 0.11.0 with 1.3 and
   0.13.0 with 1.4 are seen in real exports. Any other pair is refused until
   a fixture for it exists. Upstream `json-export-schema.md` at `v0.13.0`
   contradicts itself on 1.4 and 1.5 (see [Risks](#effort)), so the schema
   number by itself does not say which fields to expect.
5. Phase 2 adds telemetry joins from AIPerf's opt-in raw scrape file
   `server_metrics_export.jsonl` into `telemetry.v1` NDJSON. It does not use
   the aggregate `server_metrics_export.json`.
6. Phase 1 starts after #192 to #195 (B2 to B5) merge, because those issues
   add `summary.v3` fields the importer must fill or mark absent. #191 (B1)
   is already on `main` (81cf556), and its fields are mapped below. Phase 2
   follows #196 (C1), which extends `telemetry.v1` to every binary.

Effort: about 4 engineer-days for phase 1 and 2 to 3 for phase 2. Upkeep is
about half a day per AIPerf minor release (see [Effort](#effort)).

Why yes: on the same records, a recomputed Bench summary matches AIPerf's
own TTFT, E2E, and inter-chunk percentiles to floating-point noise (see
[Evidence](#evidence)). The import is therefore faithful. It also gives
AIPerf users the Bench differentiators that make sense after the fact:
`$/M` output tokens, a declared SUT block, and telemetry on the request
clock. A second benefit is that it cross-checks the Bench summarizer against
a second implementation.

Why narrow: some Bench differentiators cannot come from an AIPerf export at
all. WER/CER needs transcripts, image digests need images, and AIPerf saves
neither. An imported run is not a Bench measurement either. The `import`
block and the rules under [Provenance](#provenance-and-sut) keep that
distinction visible in every summary.

### Options considered

| Option | Verdict | Reason |
|--------|---------|--------|
| A. No ingest; run both tools and compare with `scripts/parity/` | Rejected as the end state | It stays the way to compare *counts*. It gives AIPerf users no cost or SUT provenance. |
| B. Python adapter under `scripts/` that writes `request.v3` and a Python-computed summary | Rejected | It would duplicate the type-7 percentiles, `*_unreliable` flags, throughput bins, goodput and cost in a second language, and the two would drift. The summary math stays single-sourced in `src/summary.rs`. |
| C. Rust `import aiperf` subcommand that maps records and reuses the summarizer | **Chosen** | One summarizer. Output works with `compare`, `docs/queries/analyze.py`, and every existing JSONL reader. |
| D. Import `profile_export_aiperf.json` aggregates directly into `summary.v3` | Rejected | AIPerf aggregates use different percentile sets (p1 to p99, no MAD) and different windows, so the result would be a hybrid summary. Without records, goodput, bins, and cost cannot be recomputed under Bench rules. |

Design choices made in this note (recorded per the #184 protocol):

- The importer lives in the dispatcher (`metrum-ai-bench-cli import aiperf`),
  not in `compare` and not as a new binary. `compare` takes strategic sweep
  JSON and request CSV. A data-log JSONL from the importer feeds it with no
  change.
- `request.v3` gets no AIPerf-only fields. HTTP trace timings, TTST, and the
  time-weighted blocks stay in the source files. The `import` block records
  each file's SHA-256 digest. If Track B adds a native Bench metric that one
  of those fields supplies, the importer fills it then.
- An AIPerf success that has no visible output token is imported as a
  `no_output_token` error, as Bench classifies it. This can make the imported
  error rate higher than AIPerf's. The `import` block reports the count.

## Inputs

All counts below come from real files.

| Source | AIPerf | What it holds |
|--------|--------|---------------|
| #204 parity harness, `plain`, `reasoning`, `slo` (2026-10-05) | 0.13.0, JSON export schema `1.4` | Paced mock, c=4, 64 measured + 4 warmup requests. `profile_export.jsonl`, `profile_export_aiperf.json`, `server_metrics_export.json`, `phase_manifest.json` |
| Bake-off evidence [`evidence-aiperf-bakeoff-rtxpro6000-20260924`](https://github.com/metrum-ai/bench-cli/releases/download/evidence-aiperf-bakeoff-rtxpro6000-20260924/evidence-aiperf-bakeoff-rtxpro6000-20260924.zip) (sha256 `8cc7b81965871d4204eb7ee61f1fe523ee768289eb64bac543328afb5522386f`), `raw/aiperf/c4/` inside the zip | 0.11.0, JSON export schema `1.3` | A real vLLM-served model, c=4, 16 requests, streaming chat. Same three files plus CSVs |
| Upstream docs at tag `v0.13.0` | 0.13.0 | [Profile exports](https://github.com/ai-dynamo/aiperf/blob/v0.13.0/docs/tutorials/working-with-profile-exports.md), [JSON export schema](https://github.com/ai-dynamo/aiperf/blob/v0.13.0/docs/reference/json-export-schema.md), [metrics reference](https://github.com/ai-dynamo/aiperf/blob/v0.13.0/docs/metrics-reference.md), [server metrics](https://github.com/ai-dynamo/aiperf/blob/v0.13.0/docs/server-metrics/server-metrics.md) |

Bench targets: `request.v3` and `summary.v3` in
[OUTPUT_SCHEMA.md](../OUTPUT_SCHEMA.md), definitions in
[METRICS.md](../METRICS.md), and `telemetry.v1` in
[TELEMETRY.md](../TELEMETRY.md).

### AIPerf record shape

Each `profile_export.jsonl` line holds `metadata` and `metrics`, plus
`error` on failed requests. Upstream shows `"error": null` on successes, but
neither the 0.11.0 nor the 0.13.0 success records carry an `error` key at
all, so the reader treats a missing key as null. Each entry in `metrics` is
`{value, unit}`, and `unit` is authoritative. The real files use exactly
`ms`, `tokens`, `KB`, `count`, `ratio`, `%`, and `tokens/sec/user`, and the
reader refuses any other unit. The fields seen in the real files:

- `metadata` (0.13.0): `session_num`, `x_request_id`, `x_correlation_id`,
  `root_correlation_id`, `conversation_id`, `turn_index`, `credit_issued_ns`,
  `request_start_ns`, `request_ack_ns`, `request_end_ns`, `worker_id`,
  `record_processor_id`, `benchmark_phase` (`warmup` or `profiling`),
  `phase_index`, `phase_kind`, `phase_name`, `was_cancelled`,
  `context_overflow_skip`, `agent_depth`. 0.11.0 lacks the `phase_*`,
  `root_correlation_id`, and `context_overflow_skip` fields and writes no
  warmup records.
- `metrics` (0.13.0 `reasoning`, 36 names): `request_latency`,
  `time_to_first_token`, `time_to_second_token`,
  `time_to_first_output_token`, `decode_duration`, `inter_token_latency`,
  `inter_chunk_latency` (a list), `input_sequence_length`,
  `output_sequence_length`, `output_token_count`, `reasoning_token_count`,
  `output_token_throughput_per_user`, `e2e_output_token_throughput`,
  `prefill_throughput_per_user`, `osl_mismatch_diff_pct`,
  `usage_prompt_tokens`, `usage_completion_tokens`, `usage_total_tokens`,
  `usage_reasoning_tokens`, `usage_*_diff_pct` (three), and 15 `http_req_*`
  trace timings. `slo` adds `good_request_count`. 0.11.0 has 27 names and no
  `usage_*` or `decode_duration`.
- `error`: `{code, type, message}` on failed requests, absent or null
  otherwise. For failed requests, `metrics` holds only `error_isl` (upstream
  example; no failed records exist in the real files).

Timestamps: `metadata.*_ns` are wall-clock epoch nanoseconds. Metric values
use a perf counter. In 0.13.0, `request_end_ns - request_start_ns` exceeds
`request_latency` by 0.05 to 0.27 ms on every record. In 0.11.0 the two are
equal. The importer therefore takes durations from `metrics`, and uses
metadata only to place requests in time.

## Field mapping: `request.v3`

One `request.v3` line per AIPerf record, sorted by `request_start_ns`. Status:
**exact** means the same quantity in the same units after conversion,
**derived** means computed from AIPerf fields with a Bench formula, and
**gap** means not available.

| `request.v3` field | AIPerf source | Conversion | Status |
|--------------------|---------------|------------|--------|
| `schema_version` | none | `metrum-ai-bench-cli.request.v3` | n/a |
| `run_id` | `benchmark_id` (aggregate JSON) | string as is; not a UUID | derived |
| `seq` | record order | index after sorting by `request_start_ns` across phases | derived |
| `phase` | `metadata.benchmark_phase` | `profiling` to `measure`, `warmup` to `warmup`; 0.11.0 has only `profiling` | exact |
| `endpoint` | `input_config.endpoint.urls[0]` | host:port. AIPerf records do not say which URL served them, so a run with several URLs collapses to one endpoint (gap below). `--redact-hostname` / `--require-sut` behave exactly as in a native run (today they null `environment.hostname` and leave `endpoint` as is), with no extra rule | derived |
| `started_at` / `completed_at` | `request_start_ns` / `request_end_ns` | epoch ns to ISO 8601 UTC | exact |
| `send_offset_s` | `request_start_ns` | minus the first measured `request_start_ns`, in seconds. Wall clock, not monotonic | derived |
| `latency_s` | `metrics.request_latency` | ms / 1000 | exact |
| `ttft_s` | `time_to_first_output_token` | ms / 1000. Bench TTFT is the first *visible* token; AIPerf `time_to_first_token` includes reasoning tokens and matches Bench `first_reasoning_s` instead | exact |
| `ttft_source` | `input_config.endpoint.streaming` | `stream` when streaming and `ttft_s` is set, otherwise absent | derived |
| `first_reasoning_s` | `time_to_first_token` | ms / 1000, only when `reasoning_token_count` or `usage_reasoning_tokens` is above 0 | exact |
| `first_byte_s` | `request_ack_ns - request_start_ns` | ns / 1e9. Null when `request_ack_ns` is null. Upstream defines it as "when server acknowledged the request" and says it "is only applicable to streaming requests" ([working-with-profile-exports.md](https://github.com/ai-dynamo/aiperf/blob/v0.13.0/docs/tutorials/working-with-profile-exports.md), line 121), so non-streaming imports have no first byte. AIPerf `http_req_waiting` is time to first *body* byte after the send finished, so it is not this field | derived |
| `connect_s` | `http_req_dns_lookup` + `http_req_connecting` | ms / 1000. `0` when the connection was reused, as in Bench. Not `http_req_connection_overhead`, which also adds `http_req_blocked` (time waiting for a pooled connection). Bench `connect_s` has no pool wait, so `blocked` is dropped. It was 0 in every real record | derived |
| `prefill_s` | `ttft_s`, `connect_s` | `max(0, ttft_s - connect_s)` | derived |
| `decode_s` | `latency_s`, `ttft_s` | `max(0, latency_s - ttft_s)` | derived |
| `decode_tok_s` | `completion_tokens`, `decode_s` | `completion_tokens / decode_s` | derived |
| `itl_s` | `inter_chunk_latency` list, `time_to_first_token`, `time_to_first_output_token` | ms / 1000. When TTFO equals TTFT (no reasoning), copy the list as is. Otherwise rebuild arrival times as TTFT plus the cumulative ICL sum, and keep only deltas between arrivals at or after TTFO (visible chunks) | exact (no reasoning); derived (reasoning) |
| `prompt_tokens` / `completion_tokens` / `total_tokens` | `usage_prompt_tokens` / `usage_completion_tokens` / `usage_total_tokens` | server usage, as Bench. 0 when absent | exact (0.13.0); gap (0.11.0) |
| `tokenized_prompt_tokens` / `tokenized_completion_tokens` | `input_sequence_length` / `output_sequence_length` | AIPerf client tokenizer counts (`input_config.tokenizer.name`) | exact |
| `usage_missing` | `usage_*` presence | true when the usage fields are absent (every 0.11.0 record) | derived |
| `in_flight_at_send` | intervals of all records | count of records with start at or before this send and end after it. Label it reconstructed | derived |
| `scheduled_offset_s` / `queue_delay_s` | none for `concurrency` phases | `scheduled_offset_s` absent and `queue_delay_s = 0`, as a native closed-loop run writes them. Open-loop (`request_rate`) phases export no intended send time, so the importer refuses them in phase 1 | gap (open loop) |
| `error` | `error.code`, `error.type`, `error.message` | 429 to `rate_limit`, other HTTP codes to `http_status`, timeout classes to `timeout`, the rest to `other { message }`. A success with no TTFO becomes `no_output_token` | derived |
| `partial` | `metadata.was_cancelled` | bool | exact |
| `modality_metrics.prompt_words` | `inputs.json` payload via `conversation_id` and `turn_index` | whitespace word count of the prompt | derived (optional) |
| `modality_metrics.completion_words` | none | AIPerf does not save response text | gap |
| `modality_labels` | `x_request_id` | `aiperf_x_request_id`, to join with server logs | derived |

## Field mapping: `summary.v3`

The importer builds the summary from the mapped records, so most fields
follow by construction. The table lists where each input comes from and what
cannot be filled.

| `summary.v3` field | Source | Status |
|--------------------|--------|--------|
| `attempted`, `successes`, `errors`, `error_rate`, `errors_by_type` | recomputed over `phase=measure` | recomputed; may exceed AIPerf errors by the `no_output_token` count |
| `latency_s`, `ttft_s`, `itl_s`, `tpot_s`, `connect_s`, `prefill_s`, `decode_s`, `decode_tok_s` | recomputed type-7 `DistSummary` | recomputed |
| `first_byte_s` (#191) | mapped `first_byte_s` | recomputed; `n=64` in each 0.13.0 run and `n=16` in 0.11.0 (all streaming). `n=0` for non-streaming imports, because `request_ack_ns` is null there |
| `queue_delay_s` (#191) | none; no `scheduled_offset_s` | `n=0` for every closed-loop import, which is what a native closed-loop run reports. Open loop is refused |
| `first_reasoning_s` (#191) | mapped `first_reasoning_s` | recomputed; `n=64` in 0.13.0 `reasoning`, `n=0` elsewhere |
| `isl_tokens`, `osl_tokens` (#191) | server usage, or `tokenized_*` on `usage_missing` rows | recomputed; `n=64` (0.13.0) and `n=16` (0.11.0) |
| `isl_tokens_source`, `osl_tokens_source` (#191) | per-row token source | `server_usage` for the 0.13.0 exports; `tokenizer_fallback` for 0.11.0, which has no usage. An export with usage on some rows only gives `mixed` |
| `window_seconds`, `requests_per_second`, `throughput_bins_rps` | first `request_start_ns` to the latest `request_start_ns + request_latency` over measured records | recomputed; equals AIPerf `benchmark_duration` to the microsecond in all four runs (7.737541 s in `plain`) |
| `completion_tokens_per_second`, `completion_tokens_source` | server usage; tokenizer counts when usage is missing | recomputed; 0.11.0 gives `tokenizer_fallback` |
| `price_per_hour`, `price_provenance`, `cost_per_million_output_tokens` | `--price-per-hour` or `sut.cost.price_per_hour` | added by Bench |
| `goodput` | Bench `--slo` flags if given, else `input_config.slos` converted ms to s | recomputed |
| `coordinated_omission_latency_s` | equals `latency_s` for closed-loop phases | recomputed (open loop refused) |
| `observed_concurrency.cap` | `input_config.phases[profiling].concurrency` | derived |
| `observed_concurrency.in_flight_*` | reconstructed `in_flight_at_send` | derived |
| `observed_concurrency.cap_engagement_fraction`, `acquire_count`, `wait_count` | none (AIPerf does not export credit waits per request) | gap; omit the block or set those fields null |
| `usage_missing_count`, `ttft_warning` | recomputed | recomputed |
| `per_endpoint` | one entry (see the `endpoint` row) | derived; multi-URL runs collapse |
| `config.run_id`, `config.effective_max_concurrency` | `benchmark_id`, phase concurrency | derived |
| `config.common.seed`, `tokenizer`, `slos`, `warmup_requests` | `run_info.random_seed`, `input_config.tokenizer.name`, `input_config.slos`, warmup phase `requests` | derived |
| `config.body_template` | first payload in `inputs.json`, with the prompt replaced by `{{prompt}}` | derived (optional) |
| `environment` | the importing host, which is not the measuring host | set `hostname` null; the `import` block says the measuring host is unknown |
| `sut` | `--sut` | added by Bench, declared |
| `isl_osl` | `--isl-target` / `--osl-target` if given | recomputed (optional) |
| `partial` | aggregate `was_cancelled` or `is_complete == false` | derived |
| new `import` block | see [Provenance](#provenance-and-sut) | added |

The #191 rows above come from the mapped records, as for any native run.
Fields from #192 to #195 are added to this table when they merge. Most of
them are Bench names for quantities that AIPerf already exports, such as
TTST, so the importer will map them from the source records then.

## Gaps

### AIPerf has it, Bench has no slot

These values are present in a 0.13.0 export and are dropped on import.
They stay in the source files, which the `import` block references by digest.

- Per request: `time_to_second_token` (equal to the first
  `inter_chunk_latency` entry in every record checked),
  `http_req_sending` / `waiting` / `receiving` / `duration` / `total`,
  `http_req_blocked` (pool wait; Bench keeps only `dns_lookup` +
  `connecting` as `connect_s`), `http_req_data_sent` / `received` (KB),
  `http_req_chunks_sent` / `received`, `http_req_connection_reused`,
  `prefill_throughput_per_user`, `output_token_throughput_per_user`,
  `e2e_output_token_throughput`, `osl_mismatch_diff_pct`,
  `usage_*_diff_pct` (derivable from `tokenized_*` and usage),
  `credit_issued_ns`, `x_correlation_id`, `conversation_id`, `turn_index`,
  `error_isl`.
- Run level: the time-weighted `effective_*` and `active_*` blocks,
  `tokens_in_flight`, `credit_to_start_latency`, `warmup_metrics`, the
  `p1`/`p5`/`p10`/`p25`/`p75` percentiles, `server_metrics_export.json`
  aggregates, and `telemetry_data` (GPU telemetry summaries).
- Multi-turn and agentic sessions. Phase 1 refuses records with
  `turn_index > 0`, because `request.v3` has no session key.

These are the same quantities behind the #184 count gap: 35 AIPerf
distributions against 10 for Bench, and 14 time-weighted blocks against 1.
Track B decides which of them become native Bench metrics. The importer
follows those decisions and does not widen `request.v3` itself.

### Bench has it, the AIPerf export cannot supply it

- **WER/CER** (ASR) and **image SHA-256 digests** (imagegen): AIPerf does not
  save response text or images. These stay Bench-only, which is one reason
  phase 1 covers LLM only.
- `modality_metrics.completion_words` and any metric that needs response text.
- Monotonic `send_offset_s`. AIPerf metadata uses the wall clock, so an NTP
  step during the run would skew windows and bins. The importer warns when
  sends are not monotonic in wall time.
- `scheduled_offset_s` / `queue_delay_s` for open-loop runs. AIPerf exports a
  replay schedule offset only for `fixed_schedule` phases, and only as an
  internal metric.
- Semaphore detail in `observed_concurrency` (`cap_engagement_fraction`,
  `acquire_count`, `wait_count`).
- Which endpoint served each request in a multi-URL run.
- Server usage counts in 0.11.0 exports. Cost then uses client tokenizer
  counts, and `completion_tokens_source` says `tokenizer_fallback`.
- The measuring host's `environment`. A live run's `--require-sut` also
  checks that the declared SUT is supplied *at run time*. An import can only
  attach the declaration afterwards.

### Semantic traps the mapping must handle

1. **TTFT.** AIPerf `time_to_first_token` counts reasoning tokens. Bench
   `ttft_s` is the first visible token, which is AIPerf
   `time_to_first_output_token`. In the `reasoning` scenario, AIPerf TTFT p50
   is 52.46 ms and TTFO p50 is 181.62 ms. The parity table shows the
   difference: median TTFT/E2E 0.109 for AIPerf against 0.374 for Bench.
2. **ITL.** AIPerf `inter_chunk_latency` includes reasoning chunks, while
   Bench `itl_s` holds visible chunks only. Rebuilding arrival times and
   keeping deltas at or after TTFO gives 2,396 intervals in `reasoning`.
   That is exactly the Bench `itl_s.n` of 2,396 on the same workload. The raw
   ICL count is 3,408. Without reasoning, the list is copied unchanged,
   because rebuilding through a cumulative sum adds about 1e-6 ms of
   rounding.
3. **Latency clock.** Take `request_latency`, not the metadata timestamp
   difference (0.05 to 0.27 ms longer in 0.13.0).
4. **First byte.** `request_ack_ns` (server acknowledged) maps to
   `first_byte_s`, and `http_req_waiting` does not. Upstream sets
   `request_ack_ns` only for streaming requests, so it can be null.
5. **Connect.** `http_req_connection_overhead` includes `http_req_blocked`,
   the wait for a pooled connection. Bench `connect_s` is DNS plus connect
   only, so the importer sums those two fields.
6. **Units.** Read `unit` on every value. Do not infer it from the name (for
   example `http_req_data_sent` is in `KB`).
7. **Telemetry names.** The aggregate `server_metrics_export.json` drops the
   Prometheus `_total` suffix from counters, in both versions. In the
   0.13.0 harness `telemetry` run, AIPerf wrote `vllm:generation_tokens` and
   `all_smi_energy_consumed_joules`, while Bench stored
   `vllm:generation_tokens_total` and `all_smi_energy_consumed_joules_total`
   from the same pages. The 0.11.0 file has `vllm:prompt_tokens` and
   `http_requests`. Joins and comparisons must normalize. Whether the raw
   `.jsonl` keeps the suffix is checked in phase 2 (see below).

## Provenance and SUT

An imported summary is evidence about an AIPerf run, and the output must say
so. The importer adds one block to `summary.v3` (an additive change) and one
optional field to `request.v3`:

```json
"import": {
  "tool": "aiperf",
  "tool_version": "0.13.0",
  "export_schema_version": "1.4",
  "benchmark_id": "<from export>",
  "files": [{"path": "profile_export.jsonl", "sha256": "<hex>"}],
  "clock": "wall",
  "reclassified_no_output_token": 0,
  "dropped_fields": ["time_to_second_token", "http_req_waiting", "..."]
}
```

Each imported `request.v3` line also gets `"source": "aiperf"`.

Rules:

- `--sut <file>` attaches the SUT block with its usual provenance
  (`declared`, or `mixed` from `sut init --probe`). It is still a declaration,
  now made after the run. `--require-sut` keeps its field checks and implies
  `--redact-hostname`.
- The importer does not copy `run_info.cli_command`: it can hold URLs, paths,
  and model names, and Bench does not need it.
- [RESULTS_PUBLICATION_POLICY.md](../RESULTS_PUBLICATION_POLICY.md) should state that published numbers from an
  import carry "measured by AIPerf <version>, imported by Metrum AI Bench CLI"
  next to them. That policy change belongs to the implementation issue, not
  to this note.
- The importer never edits or rewrites the AIPerf files. It reads them only.

## Cost

`cost_per_million_output_tokens = price_per_hour / (completion_tokens_per_second * 3600) * 1e6`,
with the throughput recomputed from the imported records. All figures here
are for the 0.13.0 `plain` run at $2.00/hour. Both tools count the same
3,472 output tokens, and the importer's window equals AIPerf's
`benchmark_duration` (7.737541 s). That gives $1.2381 per million output
tokens. AIPerf's own `output_token_throughput` (448.570 tokens/s) gives
$1.2385. The 0.03% difference is AIPerf's throughput denominator. 3,472
tokens at 448.570 tokens/s implies 7.7401 s, about 2.6 ms longer than its own
`benchmark_duration`. With 0.11.0 exports (no usage), cost uses tokenizer
counts and says so in `completion_tokens_source`. The 0.11.0 `c4` run gives
$6.3788 either way.

## Telemetry joins (phase 2)

| AIPerf file | Usable for a join | Notes |
|-------------|-------------------|-------|
| `server_metrics_export.json` / `.csv` | No | Aggregates per series (`stats`, `buckets`) over the profiling window. No samples. |
| `server_metrics_export.jsonl` | **Yes** | Opt-in (`--server-metrics-formats ... jsonl`). One line per scrape: `endpoint_url`, `timestamp_ns`, `request_sent_ns`, `first_byte_ns`, `metrics` (values, labels, histogram buckets). |
| `server_metrics_export.parquet` | Yes, deferred | Default format, but reading it would add a Parquet dependency to the Rust build. Revisit if operators cannot add `jsonl`. |
| `gpu_telemetry_export.jsonl` | Yes | DCGM, pynvml, or amdsmi samples. The v0.13.0 docs place the rename of NVIDIA metrics to `nvidia_*` in schema 1.4 in one place and 1.5 in another (see [Risks](#effort)). |

The mapping into `telemetry.v1` NDJSON:

- `run`: `t0_wall` = ISO time of the first measured `request_start_ns`.
  `telemetry_sources` = one source per `endpoint_url`, with
  `interval_ms` from the median scrape spacing.
- `telemetry`: `t_ns = timestamp_ns - t0`, `src` = the endpoint host,
  `metric`, `labels`, `value`, and `mtype` from the series type. Histograms
  become `histogram_bucket` rows. `scrape_ms = (first_byte_ns -
  request_sent_ns) / 1e6`.
- `request`: `t_sent_ns` / `t_done_ns` from `request_start_ns` /
  `request_end_ns` minus `t0`. Other fields as in the request mapping.

AIPerf request and scrape timestamps are both wall-clock nanoseconds from
the same process, so they share one clock. That is weaker than Bench's
monotonic epoch, but it is enough for the per-request joins in
`docs/queries/analyze.py`. Neither the harness runs nor the bake-off wrote
`.jsonl` (both used the `json csv` formats), so this mapping follows the
upstream docs only. Phase 2 starts by capturing a real `.jsonl` with
`scripts/parity/run_tele.sh` and AIPerf `--server-metrics-formats json csv jsonl`.
That capture settles the `_total` question and the label shape.

The #184 telemetry counts (Bench 1,904 values over 112 series, AIPerf 1,258
over 101) compare Bench raw rows with AIPerf aggregate leaves. A phase 2
import of the `.jsonl` would store AIPerf's raw samples as rows. The parity
harness should then count imported runs in their own column.

## Evidence

A throwaway prototype (not committed) applied the request mapping above to
the real files. It took latency from `request_latency`, placed requests by
`request_start_ns`, built `itl_s` with the visible-chunk rule, and fell back
to tokenizer counts where usage was missing. Then it compared type-7
percentiles over the mapped records with AIPerf's own aggregates in the same
directory. Every row below comes from that one computation. An earlier draft
of the prototype took latency and the window from `request_end_ns -
request_start_ns`, which trap 3 rules out. None of its figures remain here.

| Run | Quantity | Mapped (Bench rule) | AIPerf aggregate | Difference |
|-----|----------|---------------------|------------------|------------|
| 0.13.0 `plain` | TTFT p50 / p99 | 52.448268 / 54.895808 ms | 52.448268 / 54.895808 ms | 0 |
| 0.13.0 `plain` | E2E p50 / p99 | 489.621027 / 556.485114 ms | 489.621027 / 556.485114 ms | below 2e-13 ms |
| 0.13.0 `plain` | ITL p50, n = 3,408 (list copied) | 7.998889 ms | ICL 7.998889 ms | 0 |
| 0.13.0 `plain` | window | 7.737541 s | `benchmark_duration` 7.737541 s | 0 |
| 0.13.0 `plain` | `$/M` at $2/h | 1.2381 | 1.2385 (from `output_token_throughput`) | 0.03% |
| 0.13.0 `reasoning` | TTFT (TTFO) p50 | 181.620743 ms | `time_to_first_output_token` 181.620743 ms | 0 |
| 0.13.0 `reasoning` | visible ITL count | 2,396 | Bench `itl_s.n` 2,396 (same workload) | 0 |
| 0.11.0 `c4` | TTFT p50 / p99, E2E p50 / p99 | 161.683576 / 181.237820, 9179.782118 / 9197.920736 ms | identical | 0 |
| 0.11.0 `c4` | ITL p50 (list copied) | 45.446776 ms | ICL 45.446807 ms | AIPerf 0.11.0 computes pooled ICL percentiles differently; 0.13.0 matches |
| 0.11.0 `c4` | `$/M` at $2/h (`tokenizer_fallback`) | 6.3788 | 6.3788 | 0 |

Field coverage comes from the same prototype run, counted with the per-request rule from
[scripts/parity/README.md](../../scripts/parity/README.md) (non-null numeric
fields on measured `request.v3` lines, excluding `seq` and `error`):

| Run | Bench native per-request fields | Imported per-request fields | Missing vs native | Extra vs native |
|-----|---------------------------------|-----------------------------|-------------------|-----------------|
| `plain` (0.13.0) | 16 | 15 | `in_flight_at_send` (reconstructable), `modality_metrics.prompt_words` (from `inputs.json`), `modality_metrics.completion_words` (gap) | `tokenized_prompt_tokens`, `tokenized_completion_tokens` |
| `reasoning` (0.13.0) | 17 | 16 | same three | same two |
| `c4` (0.11.0) | n/a | 14 | also `decode_tok_s` (no server usage) | same two |

With `in_flight_at_send` reconstructed and `prompt_words` read from
`inputs.json`, an import fills 17 of 18 numeric fields. The 18 are the 16
native fields plus the two tokenized counts, and only
`modality_metrics.completion_words` stays empty. Summary quantities follow
from the records, so an imported run should count close to the 25 to 27
quantities of a native Bench run on `main` after #191 under
`count_points.py bench` (`plain` and `reasoning` 25, `slo` 27, per #213).
The exception is `observed_concurrency`, which loses its semaphore fields.
Phase 1 tests confirm the exact count.

## Effort

| Work | Days |
|------|------|
| Phase 1: serde model for the `profile_export.jsonl` record, unit-aware value reader, version gate on observed (`aiperf_version`, `schema_version`) pairs | 1 |
| Phase 1: mapping to `RequestRecord` (TTFO, ITL rebuild, error and `no_output_token` rules, `in_flight_at_send`, `inputs.json` join) | 1 |
| Phase 1: `import aiperf` clap subcommand, `--sut` / `--require-sut` / `--price-per-hour` / `--slo`, the `import` block and `source` field, plus `docs/OUTPUT_SCHEMA.md`, `docs/METRICS.md`, `CHANGELOG.md`, and `cargo xtask render-cli-help` | 1 |
| Phase 1: fixtures and tests. Generate fresh exports with `scripts/parity/run_pair.sh` (`TOOLS=aiperf`), trim them into `tests/fixtures/`, and assert the evidence table above to 1e-9 s. Add an `import` row to `count_points.py`. | 1 |
| **Phase 1 total** | **about 4** |
| Phase 2: `.jsonl` scrape reader, `telemetry.v1` writer reuse, `_total` normalization, a GPU telemetry `.jsonl` reader, capture fixture, tests | 2 to 3 |
| Upkeep per AIPerf minor release: refresh fixtures with the harness and widen the version gate | about 0.5 |

Phase 1 needs no new crates: `serde_json`, `chrono`, and `sha2` are already
dependencies in `Cargo.toml`. It touches no request
path code, so it carries no hot-path risk.

Risks:

- **Schema churn.** The JSON export schema went from 1.3 (0.11.0) to 1.4
  (0.13.0). Upstream
  [json-export-schema.md](https://github.com/ai-dynamo/aiperf/blob/v0.13.0/docs/reference/json-export-schema.md)
  at `v0.13.0` contradicts itself. Its prose (line 77) says "Schema 1.4 adds
  a `platform` field ... and vendor-scopes GPU telemetry metric names", but
  its version table gives 1.4 as `warmup_metrics` (line 146) and puts
  `platform` and the `nvidia_*` rename in 1.5 (line 147). Real 0.13.0
  exports carry `"schema_version": "1.4"` and do have `warmup_metrics`.
  Mitigation: gate on the observed (`aiperf_version`, `schema_version`)
  pair, refuse unknown pairs with a clear message, and keep one fixture per
  accepted pair.
- **Misattribution.** Mitigated by the `import` block, per-line `source`,
  and the publication policy line.
- **Track B collision.** B2 to B5 still change `summary.v3` (B1, #191, has
  merged and is mapped above). Starting after they merge avoids reworking
  the summary mapping.

## Not in scope

- Writing AIPerf formats from Bench (the reverse direction).
- Importing GenAI-Perf, vLLM `benchmark_serving`, or other tools. The
  `import` block's `tool` field leaves room for them.
- VLM, ASR, imagegen, embeddings, and multi-turn AIPerf runs in phase 1.
- Changing how the #204 harness counts native runs.

## Follow-up

If #184 accepts this recommendation, open two issues under Track C after
#192 to #195 merge: "feat(import): `import aiperf` for LLM per-request
exports" (phase 1) and "feat(import): AIPerf scrape `.jsonl` to telemetry.v1"
(phase 2, after #196). Each issue copies the field mapping and the evidence
table from this note as its acceptance criteria.

---

Metrum AI Bench CLI design note. Copyright (c) 2026 Metrum AI, Inc.
