<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Metric definitions

All intervals use `std::time::Instant`. ISO timestamps are metadata only.

Workload (prompt source, ISL/OSL, profiles, warmup, client headroom) is defined
in the live docs
[Performance Methodology](../external-docs/content/docs/performance-methodology.mdx)
Workload section. This page focuses on measured fields.

- **E2E latency**: response body completion minus actual send. Successful
  measure-phase requests only.
- **Coordinated-omission latency**: E2E latency plus delay between scheduled
  arrival and actual send. This is the headline open-loop latency.
- **First byte**: response headers received minus send (`first_byte_s`).
  Separates gateway/header delay from later body progress. Summary
  `first_byte_s` (seconds) is the type-7 distribution over measured successes
  that recorded it.
- **Queue delay**: actual send minus scheduled (intended) arrival
  (`queue_delay_s`, seconds): `max(0, send - scheduled_offset_s)`, both
  measured from the run-epoch `Instant`. Defined only for requests with an intended arrival
  (open loop, `--request-rate`). Closed-loop runs and strategic
  `--sweep-by concurrency` stages report `n=0`, not zero delay.
- **Connect**: HTTP connector duration for a new TCP/TLS session (`connect_s`).
  A value of `0` means the client reused a pooled connection (pool hit).
  Fresh connects include DNS plus TCP and, for HTTPS, TLS. TTFT still includes
  connect/TLS/queue by design; use `connect_s` with `first_byte_s` / `ttft_s`
  to attribute slow TTFT to network setup vs server queue/prefill.
- **Prefill (proxy)**: `prefill_s`. When `connect_s` is present,
  `max(0, ttft_s - connect_s)`; otherwise `ttft_s`. This is a client-side
  proxy, not a server engine prefill trace.
- **Decode**: `decode_s = max(0, e2e - ttft)` on streaming successes.
  `decode_tok_s = completion_tokens / decode_s` (strategic uses
  `output_tokens / decode_s`).
- **Observed concurrency**: client outstanding requests while the semaphore
  is held. Summary fields `observed_concurrency.in_flight_{mean,p50,max}` and
  `cap_engagement_fraction` (fraction of acquires that blocked on the cap).
  Optional per-request `in_flight_at_send`. In-flight values never exceed
  `cap`: a request leaves the gauge before its permit is released.
- **ISL/OSL validation**: optional `--isl-target` / `--osl-target` (or
  `--prompt-mix-report` metadata) compared to measured prompt/completion
  tokens. Summary `isl_osl` carries means, p50, and mismatch counts.
  `--fail-on-osl-mismatch` exits non-zero for publishable gates. Interact with
  `--max-tokens` / `ignore_eos`: unbounded OSL without a token cap will
  mismatch a tight target.
- **ISL/OSL tokens**: summary `isl_tokens` / `osl_tokens` are type-7
  distributions (unit: tokens) of per-request prompt and completion token
  counts over measured successes. Server `usage` wins; local tokenizer counts
  fill only rows flagged `usage_missing`. Rows with no usage and no
  `usage_missing` flag (ASR, imagegen) are skipped, never counted as zero.
  `isl_tokens_source` / `osl_tokens_source` record `server_usage`,
  `tokenizer_fallback`, or `mixed`, and are omitted when `n=0`.
  `isl_osl.length_basis` `tokenizer` is the same provenance as
  `tokenizer_fallback`; it reports `tokenizer` whenever any ISL or OSL sample
  used the tokenizer, where `*_tokens_source` would say `mixed`. Strategic
  sweep points use server usage only (no tokenizer fallback) and skip rows
  whose input and output tokens are both zero. Strategic `osl_tokens` is `n=0`
  for embeddings and rerank stages; rerank `isl_tokens` is `usage.total_tokens`
  (all input).
- **ITL**: every successive visible-output chunk timestamp delta, pooled
  across measured successes.
- **TPOT**: `(e2e - ttft) / (completion_tokens - 1)`, defined only for at
  least two completion tokens. Strategic streaming uses
  `(service_latency - ttft) / (output_tokens - 1)`.
- **User tok/s (`user_tps`)**: per-request output rate for an in-flight user
  (`completion_tokens / latency_s` on modality summaries;
  `output_tokens / service_latency_s` on strategic). The `user_tps=` SLO is a
  **minimum** rate (higher is better). Strategic stages also emit
  `users_at_slo = load * (meeting / successes)` when that SLO is set.
- **Request throughput**: measured successes divided by the explicit window.
  The window is first measured send → last measured successful completion,
  derived from monotonic `send_offset_s` (run-epoch `Instant`) plus
  `latency_s`. Wall-clock `started_at` is metadata only and must not be used
  to recompute the window (an NTP step would otherwise inflate it).
  The window excludes warmup and includes drain for requests issued during
  measurement.
- **Throughput bins**: fixed-width bins over send offsets (open-loop:
  `scheduled_offset_s`; closed-loop: `send_offset_s`). Each bin is divided by
  its **actual** width so a trailing partial bin is not under-normalized.
- **Effective max concurrency**: stamped on `summary.v3.config` as
  `effective_max_concurrency`: `--max-concurrency` when set, otherwise the
  closed-loop `--concurrency` value that caps outstanding work.
- **Token throughput**: successful server-usage tokens divided by that same
  window. Optional local tokenizer counts are separate fields.
- **Error rate**: measured failures divided by measured attempts.
- **Goodput**: measured successes satisfying every configured TTFT, TPOT,
  E2E, and `user_tps` SLO divided by the window.
- **Cost per million output tokens**: when a declared hourly price is present
  (`--price-per-hour` or `sut.cost.price_per_hour`),
  `price_per_hour / (completion_tokens_per_second * 3600) * 1e6`.
  Currency is assumed USD unless `sut.cost.currency` says otherwise; there is
  no FX conversion. Null when price or token throughput is absent, zero, or
  non-finite (including `usage_missing` that nulls token throughput).
- **WER/CER**: edit distance after normalization, divided by the normalized
  reference word/character count. `--normalizer` selects
  `whisper-english` (default), `whisper-basic`, or `none`, and the choice is
  recorded in the run configuration because scores are only comparable within
  one setting.
- **RTFx client**: audio duration divided by client request duration
  (`modality_metrics.rtfx_client` on measured-phase ASR records). This is the
  sole RTFx definition; legacy whole-run aggregates are not emitted.
- **Imagegen latency**: time until the response body bytes are fully read.
  Decode, hash, and artifact writes happen after the timer stops. Throughput
  denominators use the measured-phase window (warmup excluded).
- **VLM image payload**: the source bytes are sent unchanged, so
  `modality_metrics.image_bytes` matches the input file. Re-encoding happens
  only when `--max-image-dimension` forces a resize or `--reencode-jpeg` is
  requested; either way the payload size reflects what the server received.
  VLM honors `--system-prompt` (empty disables), `--min-tokens`, and
  `--tokenizer` like the LLM binary.

## Thinking models: TTFT vs first reasoning

- **TTFT**: first visible output delta minus send. Role and reasoning-only
  deltas do not count. Missing visible output is `no_output_token`. TTFT
  includes connection setup, TLS, and queueing by design. Non-streaming
  responses report `ttft_s: null` (undefined; never fabricated from E2E).
- **First reasoning**: first non-empty `reasoning_content`/`reasoning` delta
  minus send, reported separately from TTFT. Summary and strategic sweep
  point `first_reasoning_s` (seconds) is the type-7 distribution over
  measured successes that recorded it; `n=0` for non-thinking models. The
  strategic request CSV carries it per request.
- **Reasoning tokens** (`reasoning_tokens`, tokens): the server-reported
  count of reasoning tokens inside `completion_tokens` (#192). Read from the
  `usage` object (final usage chunk when streaming, response `usage`
  otherwise). Accepted locations, in order:
  `usage.completion_tokens_details.reasoning_tokens` (OpenAI Chat
  Completions, SGLang, DeepSeek-style servers),
  `usage.output_tokens_details.reasoning_tokens` (OpenAI Responses API shape),
  then flat `usage.reasoning_tokens`. The first non-zero value wins; `0` is
  recorded only when every reported location is `0`, so a placeholder
  `completion_tokens_details.reasoning_tokens: 0` cannot hide a real count
  elsewhere. Absent, `null`, negative, or non-integer values mean "not
  reported" (`null`), never `0`. There is no tokenizer
  fallback, so servers that do not report the field give `null` even when
  they stream reasoning deltas.
- **vLLM** (the default LLM and VLM engine, see [SERVING.md](SERVING.md))
  reports `usage.completion_tokens_details.reasoning_tokens` on chat
  completions, streaming and non-streaming, from v0.28.0
  ([vllm#45802](https://github.com/vllm-project/vllm/pull/45802)), and only
  when the server starts with `--reasoning-parser`
  ([reasoning outputs](https://docs.vllm.ai/en/latest/features/reasoning_outputs/)).
  Without a reasoning parser, or on vLLM before v0.28.0,
  `completion_tokens_details` is `null`, so Bench records `reasoning_tokens:
  null` and the summary shows `n=0`. That means "not reported", not "no
  reasoning". With a parser but a non-thinking model, vLLM reports `0`, which
  Bench records as `0`. Intermediate stream chunks can carry a placeholder
  `0`; the final usage chunk carries the count.
- **Visible completion tokens** (`visible_completion_tokens`, tokens):
  `completion_tokens - reasoning_tokens`. `null` when reasoning is not
  reported, or when `reasoning_tokens > completion_tokens` (inconsistent
  payload).
- Summary `reasoning_tokens` / `visible_completion_tokens` are type-7
  distributions over measured successes whose row carries the value;
  `reasoning_tokens_total` / `visible_completion_tokens_total` are their sums
  and are `null` when `n=0`. Strategic sweep points carry `reasoning_tokens`,
  `reasoning_tokens_total`, `visible_completion_tokens`
  (`output_tokens - reasoning_tokens`), and `visible_completion_tokens_total`
  for chat stages; embeddings and rerank
  never report reasoning.
- `completion_tokens`, OSL, `completion_tokens_per_second`, TPOT,
  `user_tps`, decode tok/s, and cost per million output tokens keep counting
  reasoning tokens as output, because they use the server's
  `completion_tokens` unchanged. `visible_completion_tokens` is the split for
  readers who need answer-only token counts.

Operator guidance for `--max-tokens`, `reasoning_effort`, and probe runs:
[REASONING_MODELS.md](REASONING_MODELS.md).

## Strategic telemetry (NDJSON)

Scraped Prometheus gauges/counters land in a separate `--ndjson` file, not
inside modality `--data-log` request rows.

- **Shared epoch**: every `*_ns` field is nanoseconds from one run-start
  `Instant`. `run.t0_wall` is ISO 8601 UTC metadata.
- **Power / energy (offline)**: prefer DCGM `DCGM_FI_DEV_POWER_USAGE` and
  `DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION` (mJ, often scaled to J at ingest), else
  `all_smi_gpu_power_consumption_watts`. Energy is counter Δ in a measured
  stage window, else trapezoid ∫ power. `j_per_output_token` divides that
  energy by successful output tokens in the stage.
- **KV / queue**: `vllm:gpu_cache_usage_perc` (or kv alias),
  `vllm:num_requests_{running,waiting}`, `vllm:num_preemptions_total` (Δ).
- **Sugar**: optional `request.telemetry_at_done` is last-seen only; time-
  weighted math must join long-format `telemetry` rows to `stage` windows.

Default smoke exporter: Metrum [all-smi](https://github.com/chetan-metrum-ai/all-smi)
fork on `http://127.0.0.1:9090/metrics`, scraped beside the serving engine's
`/metrics`. Which series land in the NDJSON is the runtime `--telemetry`
YAML, not a list compiled into the binary. Full join rules and recipes:
[TELEMETRY.md](TELEMETRY.md), [telemetry/ANALYSIS.md](telemetry/ANALYSIS.md).

Distributions report `n`, min, max, arithmetic mean, sample standard
deviation, median absolute deviation, and Hyndman-Fan type 7 p50/p90/p95/p99.
Undefined values serialize as null/absent, never measured zero. P99 is marked
unreliable when fewer than 100 samples exist.

Throughput dispersion uses fixed-width bins. Cross-run aggregation uses
sample dispersion and a seeded 10,000-resample percentile-bootstrap 95%
confidence interval.

Multi-endpoint aggregate distributions are labeled `pooled_mixture`; the same
full distributions are emitted independently per endpoint.

## Console vs JSONL

Printed end-of-run statistics come from the same `RunSummary` / `DistSummary`
values written as `summary.v3`. There is no separate nearest-rank console
estimator. The console prints First byte, Queue delay, First reasoning,
ISL tokens (source), OSL tokens (source), Reasoning tokens (with the total),
and Visible completion tokens lines only when that distribution has `n > 0`.
