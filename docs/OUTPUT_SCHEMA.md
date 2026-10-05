<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# JSONL output schema

Each line is a complete JSON object and carries `schema_version`.

## Request v3

`metrum-ai-bench-cli.request.v3` records are flushed immediately on completion:

- optional `run_id` (same UUID as `summary.config.run_id` when stamped)
- `seq`, `phase` (`warmup`, `measure`, `drain`), and `endpoint`
- ISO `started_at`/`completed_at`
- optional monotonic `send_offset_s` (seconds from the run-epoch `Instant` to
  actual send; preferred for window and closed-loop bins)
- monotonic `latency_s`, optional `ttft_s`, `first_byte_s`, `connect_s`,
  `prefill_s`, `decode_s`, `decode_tok_s`, `first_reasoning_s`, and `itl_s`
  (`ttft_s` is null for non-streaming LLM/VLM responses; it is never fabricated
  from E2E latency)
- optional `in_flight_at_send` (client outstanding count at send)
- optional `scheduled_offset_s` and `queue_delay_s`
- server usage counts plus optional `tokenized_*` counts and `usage_missing`
- `reasoning_tokens` / `visible_completion_tokens` (integer tokens, additive,
  #192) - always serialized. `reasoning_tokens` is the server-reported
  reasoning count from `usage` (final usage chunk when streaming, response
  `usage` otherwise; accepted locations in [METRICS.md](METRICS.md)).
  `visible_completion_tokens = completion_tokens - reasoning_tokens`. Both are
  `null` when the server did not report reasoning (never a fabricated `0`);
  `visible_completion_tokens` is also `null` when `reasoning_tokens` exceeds
  `completion_tokens`. Always `null` for ASR and imagegen, and for failed
  requests whose stream ended in an error (including `no_output_token`).
  `completion_tokens` is unchanged and still includes reasoning tokens.
- typed `error`, `partial`, modality-specific numeric metrics, and optional
  string `modality_labels` (e.g. imagegen `artifact_0_sha256`)

`modality_metrics` holds flat numeric values keyed by modality:

| Binary | Keys |
|--------|------|
| VLM | `image_count`, `image_bytes` (bytes actually sent per request) |
| ASR | `rtfx_client`, `wer`, `cer`, `inference_seconds_{server,client}` |
| Imagegen | `images_requested`, `images_returned`, `response_bytes`, `artifact_N_bytes` |

Imagegen artifact SHA-256 digests live in `modality_labels.artifact_N_sha256`
on `request.v3` (no parallel `imagegen.request.v1` lines).

`first_byte_s` is the monotonic elapsed time from send start until response
headers are received (after a successful `.send()`). It is distinct from
`ttft_s` (first visible user token), which includes connect/TLS/queue.

`connect_s` is the HTTP connector duration for establishing a new TCP/TLS
session (DNS + TCP + TLS). `0.0` means a pooled connection was reused. Prefill
and decode proxies: `prefill_s ≈ ttft_s - connect_s` (or `ttft_s` when connect
is absent); `decode_s = max(0, latency_s - ttft_s)`;
`decode_tok_s = completion_tokens / decode_s`.

## Summary v3

`metrum-ai-bench-cli.summary.v3` is field-additive over v2. It contains measured
attempted/success/error counts, rates, type-7 distributions, coordinated-omission-
corrected latency, throughput-bin dispersion, SLO goodput, `pooled_mixture`,
full `per_endpoint` distributions, environment metadata, and `partial`.

Additional v3 fields:

- `sut` - optional system-under-test block. Always present in JSON; `null` when
  `--sut` was not provided. Top-level `provenance` is usually `"declared"`;
  `sut init --probe` may write `"mixed"` with `field_provenance` marking local
  observed fields. The client still does not verify the remote serving host.
  Readers must treat `sut` as optional for historical summaries. Producers that
  pass `--require-sut` must supply `gpu.model`, `gpu.count` (>0),
  `driver_version`, `runtime.name`, `runtime.version`, `runtime.config`, and
  `host_os`. See `--sut`, `--require-sut`, `--redact-hostname`, and `sut init`.
- `environment.hostname` - may be `null` when `--redact-hostname` (or `--require-sut`, which implies redaction) is set (rc.5). Readers must treat `sut` and `hostname` as optional.
- `config` - effective run configuration:
  - `run_id` - UUID generated once per run
  - `effective_max_concurrency` - outstanding-request cap in force
    (`--max-concurrency`, or `--concurrency` when unset)
  - `common` - every `CommonBenchArgs` field (`seed`, `warmup_requests`,
    `request_rate`, `arrival`, `max_concurrency`, `load_balancer`, `ignore_eos`,
    `min_tokens`, `extra_body_json`, `system_prompt`, `unique_prompts`,
    `tokenizer`, `slos`, `throughput_bin_seconds`, `insecure`, optional
    `ca_cert` path) plus optional `scenario` (the modality `--scenario` label).
    Secrets are never stamped.
  - `effective_system_prompt` - system string actually sent (omitted/`null` when
    N/A or disabled). LLM chat defaults to no system message; VLM still uses its
    image-capable default unless overridden.
  - `body_template` - sanitized request skeleton with a `{{prompt}}` placeholder
    (no secrets, no raw images/audio)
  - `unique_prompt_nonce_template` - present when `--unique-prompts` is on:
    `[nonce-{run_id}-{seed}-{seq}]`
- `usage_missing_count` - measure-phase successes with `usage_missing`
- `completion_tokens_per_second` - `null` when any measured success has
  `usage_missing` without a tokenizer count to fill the gap; otherwise a rate
- `completion_tokens_source` - `"server_usage"` or `"tokenizer_fallback"` when
  the rate is present
- `prompt_tokens_total` (integer tokens, #193) - sum of prompt tokens over
  measured successes. Server `usage` wins; `tokenized_prompt_tokens` fills
  only `usage_missing` rows (same accounting as
  `completion_tokens_per_second`). Always present; `null` when no measured
  success reports tokens (ASR, imagegen) or any `usage_missing` row lacks a
  tokenizer count.
- `completion_tokens_total` (integer tokens, #193) - the completion token sum
  behind `completion_tokens_per_second`. Always present; `null` exactly when
  `completion_tokens_per_second` is `null`.
- `input_tokens_per_second` (tokens/second, #193) -
  `prompt_tokens_total / window_seconds`. Always present; `null` when
  `prompt_tokens_total` is `null`.
- `total_tokens_per_second` (tokens/second, #193) -
  `(prompt_tokens_total + completion_tokens_total) / window_seconds`. Always
  present; `null` unless both totals are non-null.
- `prefill_tps_per_user` (tokens/second, #193) - type-7 distribution of
  per-request `isl_tokens / ttft_s` over measured successes with ISL > 0 and
  TTFT > 0 (ISL per the `isl_tokens` rule below). Always present; `n=0`
  without streaming TTFT. TTFT includes connect time and server queueing;
  first-byte approximated TTFT (`--infer-ttft-from-first-byte`) also feeds it.
- `time_to_second_token_s` (seconds, #193) - type-7 distribution of
  per-request `ttft_s + itl_s[0]` over measured successes with a TTFT and at
  least one ITL sample (two content chunks). ITL is measured between SSE
  content chunks, so with multi-token chunks this is time to second chunk.
  Always present; `n=0` when no request qualifies.
- `user_tps` (tokens/second, #193) - type-7 distribution of per-request
  `completion_tokens / latency_s` over measured successes with server usage
  `completion_tokens > 0` (no tokenizer fallback). Same definition as the
  `user_tps=` SLO and strategic `SweepPoint.user_tps`. Always present; `n=0`
  when no request qualifies.
- These seven fields are top level only; `per_endpoint` entries do not carry
  them.
- `ttft_approx_count` - measured successes whose TTFT came from HTTP
  time-to-first-byte via `--infer-ttft-from-first-byte` (omitted when zero)
- `ttft_warning` - optional human-readable note when TTFT was approximated or
  left unmeasured without `--streaming`
- Per-request `ttft_source` - `"stream"` or `"first_byte_approx"` when `ttft_s`
  is present
- `price_per_hour` / `price_provenance` - declared `$/hour` from
  `--price-per-hour` (`"cli"`) or `sut.cost.price_per_hour` (`"sut"`); both
  `null` when absent. CLI overrides SUT.
- `cost_per_million_output_tokens` - `price_per_hour / (completion_tokens_per_second * 3600) * 1e6`
  when both inputs are usable; otherwise `null` (always present in JSON)
- `observed_concurrency` - optional client outstanding-request snapshot:
  `cap`, `in_flight_mean` / `in_flight_p50` / `in_flight_max`,
  `cap_engagement_fraction` (acquires that blocked on the semaphore),
  `acquire_count`, `wait_count`
- `connect_s` / `prefill_s` / `decode_s` / `decode_tok_s` - type-7
  distributions over measured successes (empty `n=0` when absent)
- `first_byte_s` / `queue_delay_s` / `first_reasoning_s` - type-7
  distributions in seconds over measured successes, built only from requests
  that recorded the field (#191). Always present; `n=0` with null stats when no
  request recorded it. `queue_delay_s` covers only requests with an intended
  arrival (open loop, `--request-rate`), so it is `n=0` for closed-loop runs.
  `first_reasoning_s` is `n=0` unless the model streamed reasoning deltas.
- `isl_tokens` / `osl_tokens` - type-7 distributions in tokens of per-request
  input (prompt) and output (completion) token counts over measured successes
  (#191). Server `usage` counts win; `tokenized_*` counts fill only rows
  flagged `usage_missing`. Rows with neither usage nor a `usage_missing` flag
  (ASR, imagegen) are skipped, never counted as zero. `n=0` when no row
  qualifies.
- `isl_tokens_source` / `osl_tokens_source` - optional string provenance for
  the matching token distribution: `"server_usage"` (every sample from server
  usage), `"tokenizer_fallback"` (every sample from tokenizer counts), or
  `"mixed"` (both). Omitted from JSON when the distribution has no samples.
  `isl_osl.length_basis` `tokenizer` is the same provenance as
  `tokenizer_fallback`; it reports `tokenizer` whenever any ISL or OSL sample
  used the tokenizer, where `*_tokens_source` would say `mixed`.
- Each `per_endpoint` entry carries the same seven fields (`first_byte_s`,
  `queue_delay_s`, `first_reasoning_s`, `isl_tokens`, `osl_tokens`,
  `isl_tokens_source`, `osl_tokens_source`) computed over that endpoint's
  measured successes with the same rules.
- `reasoning_tokens` / `visible_completion_tokens` - type-7 distributions in
  tokens over measured successes whose request row carries the value (#192).
  Always present; `n=0` with null stats when no measured success reported
  reasoning (non-thinking models, servers that omit the field, ASR,
  imagegen).
- `reasoning_tokens_total` / `visible_completion_tokens_total` - integer
  token sums over the same samples. Always present; `null` when the matching
  distribution has `n=0`.
- Each `per_endpoint` entry also carries `reasoning_tokens` and
  `visible_completion_tokens` distributions (no totals).
- `completion_tokens_per_second`, `osl_tokens`, and
  `cost_per_million_output_tokens` still count reasoning tokens as output
  (they use server `completion_tokens`).
- `isl_osl` - optional runtime ISL/OSL validation vs `--isl-target` /
  `--osl-target` or `--prompt-mix-report`: targets, tolerances, measured
  mean/p50, mismatch counts, and `length_basis` (`server_usage` or
  `tokenizer`)

Every `DistSummary` carries `p90_unreliable`, `p95_unreliable`, and
`p99_unreliable` using `percentile_unreliable(n, p)` (unreliable when
`n * (1 - p/100) < 1`).

Warmup request lines remain in the file for audit but are excluded from
summary distributions. Ctrl-C stops issuance, drains started requests, and
writes a partial summary. A hard kill may leave valid request lines without a
summary; consumers must accept that recoverable prefix.

Unversioned / legacy dual-summary objects are no longer written. Console output
and JSONL both derive from `RunSummary` / `DistSummary` (Hyndman–Fan type 7).
Historical `request.v2` / `summary.v2` lines from 0.1.82 remain readable for
regression audit (`record::accepts_audit_schema`); new campaign validation
rejects any JSONL line without `schema_version`.

## Strategic sweep points and request CSV

`metrum-ai-bench-cli-strategic` sweep points JSON (one `SweepPoint` per stage)
gains these type-7 `DistSummary` fields over measured successes in the stage
(additive, #191):

- `first_byte_s` (seconds) - requests that recorded a first-byte time.
- `queue_delay_s` (seconds) - scheduled-to-send delay. `n=0` for closed-loop
  stages (`--sweep-by concurrency`), where there is no intended arrival.
- `first_reasoning_s` (seconds) - requests that streamed a reasoning delta.
- `isl_tokens` / `osl_tokens` (tokens) - per-request input/output tokens from
  server `usage` only (no tokenizer fallback). Rows with zero input and zero
  output tokens are skipped. `osl_tokens` is `n=0` for `--kind embeddings` and
  `--kind rerank` stages, which generate no output. Rerank `isl_tokens` is the
  server's `usage.total_tokens` (query plus documents, all input).

Each is always present; `n=0` with null stats when no request qualifies.

Reasoning token fields (additive, #192), chat stages only (`--kind
embeddings` and `--kind rerank` never report reasoning):

- `reasoning_tokens` (tokens) - `DistSummary` of server-reported reasoning
  tokens over measured successes that reported it.
- `reasoning_tokens_total` (integer tokens) - sum of those samples; `null`
  when `reasoning_tokens` has `n=0`.
- `visible_completion_tokens` (tokens) - `DistSummary` of
  `output_tokens - reasoning_tokens` over the same rows, skipping rows where
  reasoning exceeds `output_tokens`.
- `visible_completion_tokens_total` (integer tokens) - sum of those samples;
  `null` when `visible_completion_tokens` has `n=0`.

Token totals, rates, and per-user latency fields (additive, #193). Server
`usage` only (no tokenizer fallback); formulas in [METRICS.md](METRICS.md).
`user_tps` already existed and is unchanged.

- `prompt_tokens_total` (integer tokens) - sum of `input_tokens` over
  measured successes that report usage (`input_tokens > 0` or
  `output_tokens > 0`). `null` when no success reports usage. Rerank sums
  `usage.total_tokens` (all input).
- `completion_tokens_total` (integer tokens) - sum of `output_tokens` over the
  same rows. `null` when no success reports usage, and always `null` for
  `--kind embeddings` and `--kind rerank` stages (no generated output).
- `input_tokens_per_second` (tokens/second) - `prompt_tokens_total` divided by
  the stage window; `null` when that total is `null`.
- `total_tokens_per_second` (tokens/second) -
  `(prompt_tokens_total + completion_tokens_total)` divided by the stage
  window; `null` unless both totals are non-null.
- `prefill_tps_per_user` (tokens/second) - `DistSummary` of per-request
  `input_tokens / ttft_s` over successes with `input_tokens > 0` and
  `ttft_s > 0`; `n=0` without streaming TTFT.
- `time_to_second_token_s` (seconds) - `DistSummary` of per-request
  `ttft_s + itl_s[0]` over successes with a TTFT and at least one ITL sample.

The strategic request CSV (`--csv`, one `BenchRecord` row per request) gains
trailing optional columns, in this order: `first_reasoning_s` (seconds), then
`reasoning_tokens` (integer tokens, #192, last column). The `first_reasoning_s`
cell is empty when the request streamed no reasoning delta; the
`reasoning_tokens` cell is empty when the server did not report reasoning.
Existing columns keep their order, and CSVs written before these columns
existed still load (missing cells read as empty).

## Security and provenance

`environment` is client-observed (OS, architecture, optional hostname). The
`sut` block is primarily operator-declared. When created with
`sut init --probe`, local host fields may be marked `observed` in
`field_provenance` while `runtime` / `model` / `vendor` stay declared. For
publication runs use `--sut <file> --require-sut` (implies `--redact-hostname`).
See [Publishing a result](../README.md#publishing-a-result).

## Strategic telemetry NDJSON (`metrum-ai-bench-cli.telemetry.v1`)

`metrum-ai-bench-cli-strategic --ndjson PATH` writes one tagged JSON object per
line. Rows are discriminated by `kind`. All `*_ns` fields are nanoseconds from
a single run-wide monotonic epoch; `run.t0_wall` is the ISO 8601 UTC wall
anchor for that Instant. See [TELEMETRY.md](TELEMETRY.md) and
[telemetry/ANALYSIS.md](telemetry/ANALYSIS.md).

| `kind` | Required fields | Optional |
|--------|-----------------|----------|
| `run` | `run_id`, `t0_wall`, `tool_version`, `schema_version` (`metrum-ai-bench-cli.telemetry.v1`), `config` | `sut`, `telemetry_sources` (`name`, `url`, `interval_ms`, `clock_offset_ms`, `matched_series`) |
| `stage` | `run_id`, `stage`, `load`, `phase` (`warmup` \| `measure`), `t_start_ns`, `t_end_ns` | |
| `telemetry` | `run_id`, `t_ns`, `src`, `metric`, `value`, `unit`, `mtype`, `scrape_ms` | `labels` (string map), `raw` (pre-scale value when `units.scale != 1`) |
| `request` | `run_id`, `seq`, `stage`, `warmup`, `t_sched_ns`, `t_sent_ns`, `t_done_ns`, `success`, `input_tokens`, `output_tokens`, `latency_s`, `queue_delay_s`, `service_latency_s` | `t_first_ns`, `ttft_s`, `error`, `reasoning_tokens` (integer tokens, server-reported; omitted when not reported, #192), `telemetry_at_done` (last-seen map; sugar, not truth) |
| `scrape_error` | `run_id`, `t_ns`, `src`, `error` | `http_status` |
| `summary` | `run_id`, `partial`, `dropped_telemetry_rows`, `request_rows`, `telemetry_rows`, `scrape_error_rows`, `stage_rows` | |

`mtype` is `counter`, `gauge`, `histogram_bucket`, `summary`, or `unknown`.
Telemetry rows may be dropped under writer backpressure; `summary.dropped_telemetry_rows`
counts those drops. Request, stage, run, scrape_error, and summary rows are
awaited and are not dropped. This file is separate from modality `--data-log`
JSONL.
