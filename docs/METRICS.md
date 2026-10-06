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
  to attribute slow TTFT to network setup vs server queue/prefill. LLM, VLM,
  ASR, imagegen, and strategic all record it (#194; before, only LLM and
  strategic did). For imagegen with retries, the HTTP phase fields describe
  the last attempt (the successful one on success).
- **Connection reused** (`connection_reused`, bool): true when no connector
  call for this request finished before its response headers, that is the
  pool supplied a live connection. When the client starts a connect and a
  pooled connection frees up first, the losing connect keeps running in the
  background: work it finishes after the headers is not booked to the row
  (`connection_reused = true`, and it adds nothing to `connect_s` or
  `dns_s`). If it finishes before the headers, the row still reads `false`
  and carries that connect and DNS time, which also lowers `prefill_s`. This
  mostly affects ramp-up and open-loop stages. Requests with no response
  headers (failures) read `false` whenever a connect was attempted. Summary `connections_reused` (integer) counts measured successes
  with `connection_reused = true`; `connection_reuse_rate` is
  `connections_reused / successes carrying the flag`. Both are `null` when no
  measured success carries the flag (#194).
- **DNS** (`dns_s`, seconds): time in name resolution inside the connector.
  The shared client installs a timed resolver that resolves with
  `getaddrinfo` on a blocking thread (tokio `lookup_host`), the same path as
  reqwest's default resolver. `0.0` when no lookup ran (pool hit or
  IP-literal host). `dns_s` is part of `connect_s`, not added to it (#194).
  When one request triggers more than one connector call (for example a
  reconnect after a stale pooled connection), both `connect_s` and `dns_s`
  sum those calls, so `dns_s <= connect_s` holds on successes. A failed
  lookup still records its time, so a DNS-failure row can show `dns_s > 0`
  with `connect_s = 0.0` (the connector never completed). A lookup cancelled
  by `connect_timeout` also keeps its elapsed time. On a row that got
  headers, resolver time counts only when its connect finished before the
  headers, so a pooled row (`connection_reused = true`) always reads
  `dns_s = 0.0`.
- **TCP and TLS are not split.** reqwest runs TCP connect and the TLS
  handshake inside one opaque connector future, and Bench times that future
  as a whole (`connector_layer`). Only their sum is observable:
  `connect_s - dns_s`. Bench does not report separate `tcp_s` / `tls_s`
  fields (#194).
- **Receive** (`receive_s`, seconds): response headers received to last
  response body chunk. Successes only (#194). Streaming clients read the body
  to its end after `data: [DONE]` (bounded at 250 ms and 64 KiB) so the
  connection returns to the pool; that tail is in `receive_s` but not in
  `latency_s`, so `first_byte_s + receive_s` can exceed `latency_s` when a
  server holds the stream open after `[DONE]`.
- **Bytes sent** (`bytes_sent`, bytes): request body bytes, headers excluded.
  Taken from the in-memory body, or from the `Content-Length` header for
  multipart bodies. Absent when the length is unknown (#194).
- **Bytes received** (`bytes_received`, bytes): response body bytes after
  content decoding (for example gzip), headers excluded. Successes only
  (#194).
- **Chunks received** (`chunks_received`, count): response body chunks as
  yielded by the HTTP client. These are transport reads, not SSE events, so
  one chunk may hold several events or part of one. Successes only (#194).
- Failed requests keep `connect_s`, `connection_reused`, `dns_s`, and
  `bytes_sent` but omit `receive_s`, `bytes_received`, and
  `chunks_received`, because an error body may not be read through the
  trace. Summary `dns_s`, `receive_s`, `bytes_sent`, `bytes_received`, and
  `chunks_received` are type-7 distributions over measured successes that
  carry the field (`n=0` when none). ASR `modality_metrics.bytes_sent` /
  `bytes_received` are unchanged and differ from these wire fields: they are
  the audio file bytes and the response text length.
- **Prefill (proxy)**: `prefill_s`. When `connect_s` is present,
  `max(0, ttft_s - connect_s)`; otherwise `ttft_s`. This is a client-side
  proxy, not a server engine prefill trace. VLM now records `connect_s`, so
  VLM `prefill_s` is `ttft_s - connect_s` like LLM (#194).
- **Decode**: `decode_s = max(0, e2e - ttft)` on streaming successes.
  `decode_tok_s = completion_tokens / decode_s` (strategic uses
  `output_tokens / decode_s`).
- **Observed concurrency**: client outstanding requests while the semaphore
  is held. Summary fields `observed_concurrency.in_flight_{mean,p50,max}` and
  `cap_engagement_fraction` (fraction of acquires that blocked on the cap).
  Optional per-request `in_flight_at_send`. In-flight values never exceed
  `cap`: a request leaves the gauge before its permit is released. Measured
  phase only: the gauge resets at the warmup barrier (per stage in
  strategic), so `in_flight_*`, `cap_engagement_fraction`, `acquire_count`
  and `wait_count` exclude warmup and `acquire_count` equals the measured
  requests dispatched (#226).
- **Time-weighted metrics** (#195): six blocks, each an object
  `{n, avg, active_avg, max, active_s}`, built by a sweep line over
  per-request intervals. The design matches AIPerf
  [effective vs active metrics](https://docs.nvidia.com/aiperf/reference/effective-vs-active-metrics):
  time-weighted averages of a step function over the benchmark window, with
  "active" variants restricted to time when the phase has a request in
  flight.
  - **Intervals**: one per measured success (failures and warmup excluded,
    like every rate in the summary): `[send, send + latency_s]`, where `send`
    is the monotonic `send_offset_s`. Strategic uses `sent_unix_ns` and
    `service_latency_s`, so client queue delay is excluded. The window starts
    at the first measured send of any outcome (the same start as
    `window_seconds`) and lasts `window_seconds`, which ends at the latest
    successful completion, so no success is clipped. Strategic uses the same
    rule over the stage rows (first measured send to latest successful
    `sent + service_latency_s`), which can differ slightly from the stage
    window behind `throughput`. Intervals are clipped to the window. AIPerf
    ends its window at the final response of any outcome; Bench ends it at
    the last successful completion, like `window_seconds`.
  - **Failures are excluded.** Under errors or timeouts the server was also
    busy with the failed requests, so `effective_concurrency` understates
    server busyness. Compare it with `observed_concurrency`, which counts
    failed requests while they are in flight.
  - **Phase split**: at the first generated token,
    `min(first_reasoning_s, ttft_s)` (the same prefill end as
    `prefill_tps_per_user`), clamped into `[0, latency_s]`. Rows with
    `ttft_source = first_byte_approx` or no TTFT (unary) have no split and
    count in `effective_concurrency` only.
  - **Block fields**: `n` (integer) is the number of requests eligible for
    the block, counted before clipping to the window.
    `avg = integral / window_seconds`. `active_avg = integral / active_s`.
    `max` is the peak instantaneous value inside the window. `active_s`
    (seconds) is the time in the window with at least one contributing
    request open. `avg`, `active_avg`, `max`, and `active_s` are `null` when
    `n = 0` or the window is not positive; `active_avg` is also `null` when
    `active_s = 0`. Never 0-filled.
  - **`effective_concurrency`** (requests): requests in flight. Unclipped,
    `avg = requests_per_second * mean(latency_s)` (Little's law). This is
    not `observed_concurrency`, the client semaphore gauge sampled at each
    send, which is unchanged.
  - **`effective_prefill_concurrency`** (requests): requests between send and
    first generated token. Includes network, connect, and server queue time.
    This prefill is not the per-request `prefill_s`: that is
    `ttft_s - connect_s` (visible TTFT, connect removed), while this split
    keeps connect time and ends at the first generated token, reasoning
    included. On streaming runs without reasoning the two differ only by
    connect time (0.18% on the parity mock).
  - **`effective_decode_concurrency`** (requests): requests between first
    generated token and completion.
  - **`tokens_in_flight`** (tokens): KV-cache occupancy proxy. During prefill
    a request holds its prompt tokens; during decode it holds prompt tokens
    plus output tokens accrued linearly from 0 at the first token to
    `completion_tokens` at completion. Needs a phase split and both prompt
    and output token accounting (server usage wins, tokenizer counts fill
    `usage_missing` rows, as for `isl_tokens` / `osl_tokens`; rows with no
    accounting are skipped).
  - **`effective_prefill_throughput`** (tokens/second): each request's prompt
    tokens spread uniformly over its prefill;
    `avg = in-window prompt tokens / window_seconds`.
  - **`effective_decode_throughput`** (tokens/second): each request's output
    tokens (`completion_tokens`, reasoning included) spread uniformly over
    its decode. Decode tokens include each request's first token, which
    arrives at the split point, so the full `completion_tokens` counts. `avg` over the full window equals output tokens of split rows
    divided by `window_seconds`, so when every success streams and has a
    positive decode time it matches `completion_tokens_per_second`;
    otherwise it is lower. `active_avg` is the rate while at least
    one request decodes (the AIPerf "active" view); `max` is the peak
    aggregate rate. For both throughput blocks, rows whose phase has zero
    length or zero tokens have no defined rate and are skipped (`n`
    excludes them).
  - **Strategic tokens**: `input_tokens` / `output_tokens` from server usage
    only; rows reporting neither are skipped. Embeddings, rerank, and
    imagegen stages have no output, so `tokens_in_flight` and `effective_decode_throughput`
    are `n=0` for them.
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
  for embeddings, rerank, and imagegen stages; rerank `isl_tokens` is `usage.total_tokens`
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
  Summary `user_tps` (tokens/second, #193) is the type-7 distribution of
  `completion_tokens / latency_s` over measured successes with
  `completion_tokens > 0` and `latency_s > 0`. It uses server usage
  `completion_tokens` only (no tokenizer fallback), so `usage_missing` rows
  contribute no sample. This is the same per-request value the `user_tps=` SLO
  tests, and the modality counterpart of strategic `SweepPoint.user_tps`.
  For thinking models the numerator counts reasoning plus visible tokens,
  because server `completion_tokens` includes reasoning tokens (#192).
- **Prefill tok/s per user (`prefill_tps_per_user`)**: per-request
  client-observed prefill rate, tokens/second (#193):
  `prefill_tps_per_user = isl_tokens / first_token_s`, where
  `first_token_s = min(first_reasoning_s, ttft_s)` (just `ttft_s` when the
  request streamed no reasoning delta). The denominator is the first generated
  token of any kind: for a thinking model, visible `ttft_s` also covers the
  whole reasoning phase, which would understate prefill speed. On modality summaries the ISL
  follows the `isl_tokens` rule (server usage `prompt_tokens` wins, tokenizer
  count fills `usage_missing` rows, rows with no token accounting are
  skipped); strategic uses `input_tokens` from server usage only, with the
  same denominator. Only rows with ISL > 0 and TTFT > 0 contribute, so the distribution is
  `n=0` without streaming TTFT. TTFT includes connect time, TLS, and queueing
  on the server, so this is a lower bound on engine prefill speed, not a
  server prefill trace. Rows whose TTFT was approximated from first byte
  (`--infer-ttft-from-first-byte`, `ttft_source = first_byte_approx`) are
  excluded: headers can arrive before prefill ends, which would inflate the
  rate. It deliberately does not use `prefill_s` (`ttft_s - connect_s`): a
  TTFT-based rate stays comparable to AIPerf's per-user prefill throughput,
  and `prefill_s` exists only where connect timing is installed.
- **Time to second token (`time_to_second_token_s`)**: per-request
  `time_to_second_token_s = ttft_s + itl_s[0]`, seconds (#193). Only rows with
  a TTFT and at least one ITL sample (two visible content chunks) contribute.
  ITL is measured between SSE content chunks, so when a server packs several
  tokens into one chunk this is the time to the second chunk, not strictly
  the second token. It counts visible content chunks only: reasoning deltas
  never enter `ttft_s` or `itl_s`. Same definition on modality summaries and strategic sweep
  points.
- **Request throughput**: measured successes divided by the explicit window.
  The window is first measured send → last measured successful completion,
  derived from monotonic `send_offset_s` (run-epoch `Instant`) plus
  `latency_s`. Wall-clock `started_at` is metadata only and must not be used
  to recompute the window (an NTP step would otherwise inflate it).
  The window excludes warmup and includes drain for requests issued during
  measurement.
- **Warmup barrier** (#226): with `--warmup-requests N`, measured requests
  start only after all N warmup requests have completed (success or error),
  so measurement sees a warmed, drained server. This holds in llm, vlm, asr,
  imagegen and in every strategic stage. Every measured `t_sent_ns` is at or
  after the last warmup `t_done_ns`, and the `warmup` and `measure` NDJSON
  stage windows never overlap. Open loop (`--request-rate`): the measured
  schedule shifts by the time the barrier added. The first measured request
  is due when warmup drains, and the seeded inter-arrival gaps are kept, so
  measured requests do not burst to catch up and `queue_delay_s` does not
  count warmup time. Measured `scheduled_offset_s` / `t_sched_ns` carry the
  shifted value on the run clock. Strategic restarts its measure clock per
  stage instead.
- **Throughput bins**: fixed-width bins over send offsets (open-loop:
  `scheduled_offset_s`; closed-loop: `send_offset_s`), measured from the
  first measured request's offset. Each bin is divided by
  its **actual** width so a trailing partial bin is not under-normalized.
- **Effective max concurrency**: stamped on `summary.v3.config` as
  `effective_max_concurrency`: `--max-concurrency` when set, otherwise the
  closed-loop `--concurrency` value that caps outstanding work.
- **Token throughput**: successful server-usage tokens divided by that same
  window. Optional local tokenizer counts are separate fields.
- **Token totals and rates** (#193): summary `prompt_tokens_total` and
  `completion_tokens_total` (integer tokens) sum per-request prompt and
  completion tokens over measured successes. Server `usage` wins; tokenizer
  counts fill only `usage_missing` rows. This is the accounting behind
  `completion_tokens_per_second`:
  - `completion_tokens_per_second = completion_tokens_total / window_seconds`
  - `input_tokens_per_second = prompt_tokens_total / window_seconds`
  - `total_tokens_per_second = (prompt_tokens_total + completion_tokens_total) / window_seconds`

  A total is `null` when no measured success reports tokens (ASR, imagegen)
  or when any `usage_missing` row lacks a tokenizer count; its rate is then
  `null` too. `completion_tokens_total` is `null` exactly when
  `completion_tokens_per_second` is `null`. `total_tokens_per_second` is
  `null` unless both totals exist. Strategic sweep points carry the same four
  fields per stage from server usage only (no tokenizer fallback), summed over
  successes that report usage (`input_tokens > 0` or `output_tokens > 0`) and
  divided by the stage window. Strategic `completion_tokens_total` (and so
  `total_tokens_per_second`) is `null` for stages that generate no output
  (`--kind embeddings`, `--kind rerank`, `--kind imagegen`); rerank `prompt_tokens_total` sums
  `usage.total_tokens` (all input). A strategic total whose field sums to 0
  (no row reported it) is `null`, as on the summary. The pre-existing
  strategic `completion_tokens_per_second` is unchanged: it reads `0.0`, not
  `null`, for any stage with successes but no reported output tokens
  (including embeddings and rerank), so it can be `0.0` while
  `completion_tokens_total` is `null`.
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
- **Strategic modality sweeps** (`--kind vlm|asr|imagegen`, #197): requests
  are built with the same library builders as the modality binaries.
  `SweepPoint.modality_metrics` holds one type-7 distribution per key over
  measured successes, with the same names and definitions as modality
  `request.v3` `modality_metrics`: VLM `image_count` (count) and
  `image_bytes` (bytes); ASR `wer`, `cer` (ratios, as above),
  `rtfx_client` (`audio_duration_s / service_latency_s`), and
  `audio_duration_s` (seconds); imagegen `images_requested` and
  `images_returned` (count). A key is `n=0` when no measured success has a
  value (no `--ground-truth` for WER/CER, no duration in the sample row for
  `audio_duration_s` / `rtfx_client`). Imagegen `image_digests` counts
  decoded `b64_json` images (`images`) and distinct SHA-256 digests
  (`distinct`) over measured successes; both are 0 for `url` responses.
  - Timing: ASR audio and VLM images are loaded before the first request, so
    file I/O is not in latency. For asr and imagegen the sweep clock runs
    from send until the response body is fully read; parsing, WER/CER and
    image decode/hash run after it, after the concurrency slot is released,
    on the blocking pool. That matches the imagegen binary exactly. The ASR
    binary's clock also covers building the multipart form and parsing the
    response, so a sweep's ASR `service_latency_s` can read slightly lower
    and its `rtfx_client` slightly higher than the binary's for the same
    server. An unparseable ASR body or an undecodable `b64_json` image fails
    the request, as in the binaries. Chat, embeddings, rerank and vlm keep
    parsing inside the window, unchanged.
  - `images_requested` and the decode choice come from the body actually
    sent, so an `--extra-body-json` override of `n` or `response_format` is
    recorded as sent.
  - WER/CER use the ASR binary's scorer, so an empty normalized reference
    scores `0.0` against an empty transcript and `1.0` otherwise in both.
  - `rtfx_client` needs a positive `duration` in the sample row, in sweeps
    and in the ASR binary: a sample with `duration` 0 records neither
    `audio_duration_s` nor `rtfx_client`.
  - Tokens: vlm and asr stages report `osl_tokens` and
    `completion_tokens_total` from server usage (asr only when the server
    reports usage); imagegen generates no tokens, so `osl_tokens` is `n=0`
    and `completion_tokens_total` is `null`, as for embeddings and rerank.
    TTFT is measured only for streaming chat and vlm; asr and imagegen
    stages report `ttft_s` with `n=0`.

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

## Run telemetry (NDJSON)

Scraped Prometheus gauges/counters land in a separate `--ndjson` file, not
inside modality `--data-log` request rows. Strategic and, since #196, the
LLM, VLM, ASR, and imagegen binaries write the same
`metrum-ai-bench-cli.telemetry.v1` rows, so the derived metrics below apply
to a modality run's single `measure` stage as well.

- **Shared epoch**: every `*_ns` field is nanoseconds from one run-start
  `Instant`, shared by request and telemetry rows. `run.t0_wall` is ISO 8601
  UTC metadata.
- **Modality request timing**: a modality `request` row is derived from its
  `request.v3` record: `t_sent_ns` is `send_offset_s` (same origin: the run
  clock starts at the NDJSON epoch),
  `t_done_ns = t_sent_ns + latency_s`, `service_latency_s` is the record's
  `latency_s`, and the row's `latency_s = queue_delay_s + latency_s`
  (seconds). The `measure` stage window runs from the first measured send to
  the last measured completion. Mapping:
  [OUTPUT_SCHEMA.md](OUTPUT_SCHEMA.md#modality-binaries).
- **Summary telemetry counts**: `summary.v3.telemetry` (present only with
  `--ndjson`) reports integer row counts per kind and
  `dropped_telemetry_rows`; a non-zero drop count means time-weighted
  telemetry math in that run has gaps. Strategic sweep points do not carry
  this block; strategic stdout reports `dropped_telemetry_rows` once per run.
- **Power / energy (offline)**: one GPU power source and one energy counter
  per stage, never summed across exporters. Prefer the Metrum all-smi fork
  (`all_smi_gpu_power_consumption_watts`,
  `all_smi_gpu_energy_hw_millijoules_total` in mJ), else DCGM
  (`DCGM_FI_DEV_POWER_USAGE`, `DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION` in mJ,
  often scaled to J at ingest). Chassis, node, IPMI and Redfish meters are wall
  power and never count as GPU power. Energy is counter Δ in a measured
  stage window, else trapezoid ∫ power (also when the energy counter resets
  inside the window). `j_per_output_token` divides that energy by successful
  output tokens in the stage.
- **KV / queue**: `vllm:gpu_cache_usage_perc` (or kv alias),
  `vllm:num_requests_{running,waiting}`, `vllm:num_preemptions_total` (Δ).
- **Derived GPU / KV metrics (offline)**: per measured stage,
  `docs/queries/analyze.py` computes `gpu_util_mean`, `sm_active_p50`,
  `sm_occupancy_p50`, `tensor_active_p50`, `hollow_util_mean`,
  `kv_cache_util_mean` (ratios in [0, 1]), `preemptions_delta` (count), and
  engine histogram p50/p95 (seconds, Prometheus `histogram_quantile`
  interpolation over bucket deltas; a rank in the first finite bucket or in
  `+Inf` is not interpolated and is reported as null with a bound and a
  reason, `below_first_bucket` or `above_last_bucket`, #231). Given the
  strategic stdout JSON it also reports `kv_cache_util_at_knee`. A metric
  with no source series in the window, or a counter that resets, is null,
  never 0. These are computed offline from the NDJSON and are not fields of
  any bench-cli record.
  Definitions, source series, and DCGM fallbacks:
  [telemetry/ANALYSIS.md](telemetry/ANALYSIS.md#derived-metrics-per-measured-stage) (#199).
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
It also prints Time to second token, Prefill tok/s per user, and User tok/s
lines when that distribution has `n > 0`, and Prompt tokens with Input
tokens/sec, Completion tokens, and Total tokens/sec lines when the value is
not `null` (#193). It prints one line per time-weighted block (Effective
concurrency, Effective prefill concurrency, Effective decode concurrency,
Tokens in flight, Effective prefill tok/s, Effective decode tok/s) when that
block has `n > 0` (#195).
