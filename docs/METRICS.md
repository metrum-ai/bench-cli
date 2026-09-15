<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Metric definitions

All intervals use `std::time::Instant`. ISO timestamps are metadata only.

- **E2E latency**: response body completion minus actual send. Successful
  measure-phase requests only.
- **Coordinated-omission latency**: E2E latency plus delay between scheduled
  arrival and actual send. This is the headline open-loop latency.
- **TTFT**: first visible output delta minus send. Role and reasoning-only
  deltas do not count. Missing visible output is `no_output_token`.
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
- **WER/CER**: edit distance after shared Whisper-English normalization,
  divided by normalized reference word/character count.
- **RTFx client**: audio duration divided by client request duration.

Distributions report `n`, min, max, arithmetic mean, sample standard
deviation, median absolute deviation, and Hyndman-Fan type 7 p50/p90/p95/p99.
Undefined values serialize as null/absent, never measured zero. P99 is marked
unreliable when fewer than 100 samples exist.

Throughput dispersion uses fixed-width bins. Cross-run aggregation uses
sample dispersion and a seeded 10,000-resample percentile-bootstrap 95%
confidence interval.

Multi-endpoint aggregate distributions are labeled `pooled_mixture`; the same
full distributions are emitted independently per endpoint.
