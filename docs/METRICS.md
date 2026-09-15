<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Metric definitions

All intervals use `std::time::Instant`. ISO timestamps are metadata only.

- **E2E latency**: response body completion minus actual send. Successful
  measure-phase requests only.
- **Coordinated-omission latency**: E2E latency plus delay between scheduled
  arrival and actual send. This is the headline open-loop latency.
- **TTFT**: first visible output delta minus send. Role and reasoning-only
  deltas do not count. Missing visible output is `no_output_token`. TTFT
  includes connection setup, TLS, and queueing by design. Non-streaming
  responses report `ttft_s: null` (undefined; never fabricated from E2E).
- **First byte**: response headers received minus send (`first_byte_s`).
  Separates gateway/header delay from prefill. `connect_s` is not yet
  recorded (deferred post-v1).
- **First reasoning**: first non-empty `reasoning_content`/`reasoning` delta
  minus send, reported separately from TTFT.
- **ITL**: every successive visible-output chunk timestamp delta, pooled
  across measured successes.
- **TPOT**: `(e2e - ttft) / (completion_tokens - 1)`, defined only for at
  least two completion tokens.
- **Request throughput**: measured successes divided by the explicit window.
  The window excludes warmup and includes drain for requests issued during
  measurement.
- **Token throughput**: successful server-usage tokens divided by that same
  window. Optional local tokenizer counts are separate fields.
- **Error rate**: measured failures divided by measured attempts.
- **Goodput**: measured successes satisfying every configured TTFT, TPOT, and
  E2E SLO divided by the window.
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
estimator.

