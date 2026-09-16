<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Known limitations (1.0)

Honest scope for Metrum AI Bench at the 1.0 line. See also
[COMPARISON.md](COMPARISON.md) and [METRICS.md](METRICS.md).

## Client-side only

Bench is a load-generation **client**. It measures what the client observes
(send, first byte, first visible token, body complete, errors). It cannot
observe GPU utilization, KV-cache state, or scheduler internals on the server
except when you optionally scrape a metrics URL from the strategic runner.

## SUT is declared, not verified

`--sut` / `--require-sut` (when present) embed an operator-supplied system-under-test
block into the run summary for publication. They do **not** probe the remote
host, confirm GPU SKU, driver, or engine version, or prove that the declared
SUT matches the endpoint you hit. A mismatched or empty declaration is a
policy/process failure, not something the client can detect.

## Performance, not quality (except ASR)

Headline outputs are latency, throughput, goodput under SLOs, and related
distributions. The only built-in **quality** scores are ASR **WER/CER** (with
`--normalizer`). There is no general task accuracy, judge score, or agent
success metric in 1.0.

## No agent mode

Multi-turn / tool / schema paths on strategic are request validity helpers,
not an agent evaluation harness.

## No cost per accepted task

Token, dollar, or “accepted task” cost accounting is not in 1.0 (roadmap:
next release, no date).

## Open-loop and closed-loop

Both modes exist: omit `--request-rate` for closed-loop concurrency; set
`--request-rate` (and optional `--arrival poisson`) for open-loop. Open-loop
coordinated-omission latency is the headline field when scheduling is open-loop.
Do not compare open-loop and closed-loop numbers as if they were the same
experiment.

## Pooled multi-endpoint = mixture

When requests are distributed across endpoints, aggregate distributions are
labeled `pooled_mixture`. Use `per_endpoint` for diagnosis; do not treat the
pool as a single homogeneous replica.

## p99 unreliability

p99 is marked `p99_unreliable` when fewer than **100** samples exist
(`src/stats.rs`). Small-*n* p95/p90 also carry unreliability flags; treat tail
percentiles accordingly.

## Single node / single client process

One Bench process drives one endpoint or one configured pool. Fleet-wide
comparison, history, and attestation are Platform concerns, not this OSS
client.

## Published smoke matrix is NVIDIA-only

See [SMOKE_RESULTS.md](SMOKE_RESULTS.md). AMD Instinct coverage is in progress.
No comparative vendor results are published there.

## MLPerf export is unofficial

`--mlperf-dir` writes parser-oriented LoadGen-shaped files marked **UNOFFICIAL**
and never emits bare `Result is : VALID`. It is not a submission. See
[STRATEGIC_BENCHMARKING.md](STRATEGIC_BENCHMARKING.md).

## Partial runs

Interrupted runs can write `partial: true`. Consumers must not treat a partial
summary as a complete campaign cell without checking that flag and sample
counts.
