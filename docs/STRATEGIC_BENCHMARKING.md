<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Strategic benchmarking

For agent-bench, start from the streaming LLM/chat path so TTFT measures visible
output as it arrives. The unary strategic path cannot measure TTFT. Strategic
chat turns opt in with `--streaming`, including `--sessions`; their per-turn CSV
records include `first_byte_s` and `ttft_s`, and measured TTFT enforces `--slo ttft=`.
Streaming is off by default. Embeddings and rerank remain JSON, and `--tools`
cannot be combined with `--streaming`.


`metrum-ai-bench-strategic` is the runner for concurrency/rate sweeps, chat
sessions, structured output, embeddings, reranking, server correlation and
portable exports. Existing modality-specific binaries remain supported.

## Sweep and server correlation

```bash
metrum-ai-bench-mock-server --listen 127.0.0.1:8080 &
metrum-ai-bench-strategic \
  --url http://127.0.0.1:8080/v1/chat/completions \
  --model mock --sweep 1,2,4,8,16 --sweep-by concurrency \
  --requests-per-stage 100 \
  --metrics-url http://127.0.0.1:8080/metrics \
  --html report.html --csv requests.csv \
  --mlperf-dir mlperf --mlperf-scenario server
```

Each load stage is measured independently. The report plots achieved
throughput against p95 latency and marks the unit-normalized Kneedle result.
The metrics scraper recognizes vLLM, SGLang and TensorRT-LLM names for
KV-cache utilization, preemptions, and running/waiting queues.

Use `--sweep-by rate --max-in-flight N` for open-loop request-rate stages.
Rate requests retain their intended schedule while waiting for an in-flight
slot. CSV fields include scheduled and sent timestamps, queue delay,
send-to-completion service latency, and scheduled-to-completion latency. Sweep
percentiles use the repository-wide Hyndman-Fan type 7 estimator over the last
value, so overload cannot hide behind coordinated omission. Each sweep point
carries `n`, `errors`, a full `latency_s` DistSummary (including reliability
flags), redacted stage `config`, and `goodput`.

Use repeatable `--slo e2e=…` so goodput counts only schema-valid successes that
also meet the end-to-end latency threshold. Without `--slo`,
`goodput_equals_throughput` is true and goodput is validity-filtered throughput
(often identical to throughput when no validity checker is configured).
`ttft=` / `tpot=` are accepted for CLI parity but ignored here because strategic
records do not carry those timings.

The HTTP client is shared across stages (warm connection pool).

The CSV contains one row per request; HTML is self-contained (inline SVG and
CSS). MLPerf LoadGen-style exports contain `mlperf_log_summary.txt`,
`mlperf_log_detail.txt`, and `mlperf_log_accuracy.json` for Server or Offline
scenarios. Every export file begins with an **UNOFFICIAL** disclaimer; the
summary never prints a bare `Result is : VALID` without that disclaimer.
This export is parser-oriented interoperability and is not an audited or
submitted MLPerf result; official submissions must execute the MLPerf LoadGen
and compliance suite.

## Multi-turn and structured output

Session input is JSONL:

```json
{"session_id":"support-1","messages":[{"role":"user","content":"My order is late"},{"role":"assistant","content":"What is the order ID?"},{"role":"user","content":"A-42"}]}
```

Every successive turn is submitted with its preceding history.
`--shared-prefix TEXT --prefix-control shared` preserves a cacheable prefix;
`unique` adds a stable per-session discriminator; `none` omits it.

`--json-schema schema.json` requests strict JSON-schema output and counts
syntactically valid objects containing every required property.
`--tools tools.json` requests a tool call and checks the selected function name
and JSON arguments. Valid responses determine `validity_rate` and feed goodput
(together with optional `--slo`).

## Embeddings and reranking

Use `--kind embeddings` with `--prompt TEXT` against `/v1/embeddings`.
Use `--kind rerank` with `--prompt 'query|document one|document two'` against
Jina/Cohere-style `/v1/rerank` endpoints.

## OpenTelemetry

OTLP export is opt-in at build and runtime:

```bash
cargo build --release --features otlp
OTEL_EXPORTER_OTLP_HEADERS='authorization=Bearer token' \
metrum-ai-bench-strategic ... --otlp-endpoint https://collector.example.com
```

The exporter emits standard OTLP/HTTP JSON spans to `/v1/traces` and summary
request counters plus p95 latency to `/v1/metrics`. Spans cover intended
schedule through completion and carry queue and service latency attributes.
No network telemetry occurs unless `--otlp-endpoint` is supplied.

## Mock server

`metrum-ai-bench-mock-server` is a deterministic Rust fixture supporting health,
Prometheus metrics, chat/completions, embeddings, reranking, tool calls and
JSON-schema-shaped output. `--latency-ms` controls delay and `--fail-every N`
injects reproducible HTTP 503 responses. The existing Go dummy server remains
the deeper compatibility fixture.

It is shipped as a binary target in the published crate:

```bash
cargo install metrum-ai-bench --bin metrum-ai-bench-mock-server
```

## Distribution

The crate is publishable with `cargo publish`. Tagged releases cross-build Linux
gnu (glibc, not musl) and macOS Darwin archives with a pinned
`cargo-zigbuild` container on Linux, generate CycloneDX SBOMs, checksums and
keyless Sigstore signatures, smoke-test each archive on a matching native
runner, then create the GitHub Release. The crates.io upload runs only when the
`CRATES_IO_TOKEN` secret is set, and the generated formula is committed to a tap
only when `HOMEBREW_TAP_REPOSITORY` and `HOMEBREW_TAP_TOKEN` are configured;
both are skipped with a warning otherwise, so the release itself still succeeds.
Manual dispatch requires a version-matching release tag and has separate
publication switches. `packaging/homebrew/metrum-ai-bench.rb` is the formula
template; release automation fills all platform checksums.
