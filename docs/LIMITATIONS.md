<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Known limitations (1.0)

Honest scope for Metrum AI Bench at the 1.0 line. See also
[METRICS.md](METRICS.md).

## Client-side only

Bench is a load-generation **client**. It measures what the client observes
(send, first byte, first visible token, body complete, errors). It cannot
observe GPU utilization, KV-cache state, or scheduler internals on the server
except when you optionally scrape Prometheus endpoints from the strategic
runner (`--metrics-url` and/or `--telemetry` YAML into `--ndjson`). Those
series come from exporters on the host (default: Metrum all-smi fork `/metric`),
not from an in-process NVML binding.

## Gateways that synthesize streaming

Gateways that synthesize SSE from unary upstream calls report total latency
as TTFT. This behavior is undetectable client-side.

## SUT is declared (or locally probed), not remotely verified

`--sut` / `--require-sut` embed a system-under-test block into the run summary
for publication. `metrum-ai-bench-cli sut init` writes a template;
`sut init --probe` may fill **local** GPU/OS/CPU/memory via nvidia-smi and
`/proc`, marking those fields in `field_provenance` as `observed` and setting
top-level `provenance` to `mixed`. Probing never SSHs to the remote serving
host, does not confirm that the endpoint matches the declaration, and does not
replace operator-owned `runtime` / `model` / `vendor` fields. `--require-sut`
rejects incomplete declarations (missing GPU, driver, runtime, or host OS)
before any request is sent. Values remain declared (or locally probed), not
remotely verified.

## Performance, not quality (except ASR)

Headline outputs are latency, throughput, goodput under SLOs, and related
distributions. The only built-in **quality** scores are ASR **WER/CER** (with
`--normalizer`). There is no general task accuracy, judge score, or agent
success metric in 1.0. When `sut.model.quantization` is set, the CLI prints a
stderr warning that the run measures performance, not answer quality. For
thinking-model timing (TTFT vs first reasoning, `--max-tokens`,
`reasoning_effort`), see [REASONING_MODELS.md](REASONING_MODELS.md).

## No agent mode

Multi-turn / tool / schema paths on strategic are request validity helpers,
not an agent evaluation harness. That means Bench does not score agent
**workloads under test**. It does not forbid driving the CLI from a coding
agent (Claude Code, Codex, OpenCode, and similar); see the docs-site
[Agent-driven benchmarking](https://docs.metrum.ai/metrum-ai-bench-cli/latest/docs/agent-driven/)
page.

## Cost is declared, not a full TCO

Optional `--price-per-hour` / `sut.cost.price_per_hour` yields
`cost_per_million_output_tokens` from measured output-token throughput. That is
a simple `$ / 1M output tokens` conversion, not total cost of ownership
(network, storage, multi-node, idle time, or currency FX). Absent price leaves
the field `null`. There is still no "cost per accepted task" quality gate.

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

See [SMOKE_RESULTS.md](SMOKE_RESULTS.md) for the NVIDIA-only smoke matrix from
package `1.0.0-rc.1`. AMD Instinct coverage is in progress. That is coverage,
not a vendor ranking.

## MLPerf export is unofficial

`--mlperf-dir` writes parser-oriented LoadGen-shaped files marked **UNOFFICIAL**
and never emits bare `Result is : VALID`. It is not a submission. See
[STRATEGIC_BENCHMARKING.md](STRATEGIC_BENCHMARKING.md).

## Partial runs

Interrupted runs can write `partial: true`. Consumers must not treat a partial
summary as a complete campaign cell without checking that flag and sample
counts.

## Prompt-library mixes vs llm scheduling

`metrum-ai-bench-cli-prompts` can solve for ISL/OSL mean or median within
tolerances, including by repeating rows or leaving the preferred `--count`.
`metrum-ai-bench-cli-llm` still shuffles `--prompts` and cycles with modulo
indexing under a **global** `--max-tokens`. The selected mix is preserved only
when `--num-requests` equals the extractor's `selected_count` and
`--warmup-requests` is `0`. Warmup or a mismatched request count changes the
measured mix. Repeats in the JSONL are solver output for length statistics,
not a claim about prompt diversity or answer quality. See
[PROMPT_LIBRARY.md](PROMPT_LIBRARY.md).
