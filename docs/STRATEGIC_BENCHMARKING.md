<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Strategic benchmarking

For agent-bench, start from the streaming LLM/chat path so TTFT measures visible
output as it arrives. The unary strategic path cannot measure TTFT. Strategic
chat turns opt in with `--streaming`, including `--sessions`; their per-turn CSV
records include `first_byte_s` and `ttft_s`, and measured TTFT enforces `--slo ttft=`.
Streaming is off by default. Embeddings and rerank remain JSON, and `--tools`
cannot be combined with `--streaming`.


`metrum-ai-bench-cli-strategic` is the runner for concurrency/rate sweeps, chat
sessions, structured output, embeddings, reranking, server correlation and
portable exports. Existing modality-specific binaries remain supported.

## Sweep and server correlation

```bash
metrum-ai-bench-cli-mock-server --listen 127.0.0.1:8080 --telemetry-fixture &
metrum-ai-bench-cli-strategic \
  --url http://127.0.0.1:8080/v1/chat/completions \
  --model mock --api-key dummy --sweep 1,2,4,8,16 --sweep-by concurrency \
  --requests-per-stage 100 \
  --max-tokens 64 \
  --warmup-requests 4 \
  --sut examples/sut.example.json --require-sut \
  --ndjson run.ndjson \
  --telemetry docs/telemetry/examples/all-smi.yaml \
  --metrics-url http://127.0.0.1:8080/metrics \
  --html report.html --csv requests.csv \
  --mlperf-dir mlperf --mlperf-scenario server
```

Publishable sweeps need `--sut` (and typically `--require-sut`) so the
machine-readable summary and HTML carry a SUT block under
`docs/RESULTS_PUBLICATION_POLICY.md`. Without `--sut`, the CLI prints the same
self-describing notice as the modality binaries.

Each load stage is measured independently. `--warmup-requests` are issued and
fully completed at the start of every stage, then the measurement epoch resets.
Warmup rows are written to the CSV with `warmup=true` and excluded from stage
`n`, latency percentiles, throughput, goodput, and knee detection. Measured
prompt indexing restarts at zero after warmup so the mix is not shifted.
Prefer a warmup count at least as large as stage concurrency on GPU endpoints
so cold model-load and CUDA graph capture do not inflate the baseline stage.
`--warmup-requests 0` is for mock/determinism only.

For fixed-length throughput studies on engines that honor them, pass
`--ignore-eos` and optionally `--min-tokens` with `--max-tokens` (engine
extensions, not portable OpenAI fields). Prefer `--extra-body-json` when a
gateway needs a different nesting. These controls are chat-only and are
stamped into stage `config`. Do not use them as defaults for natural-EOS,
tool-call, JSON-schema, or reasoning workloads.

Stdout JSON includes additive publication fields (`schema_version`,
`tool_version`, `environment`, `config`, `sut`) while retaining `points` for
`metrum-ai-bench-cli compare`.

For a Hugging Face prompt-library mix, extract JSONL with
`metrum-ai-bench-cli-prompts`, then pass the file and the report's
`recommended_max_tokens`:

```bash
metrum-ai-bench-cli-strategic \
  --url http://127.0.0.1:8080/v1/chat/completions \
  --model mock --streaming \
  --prompts /tmp/mix.jsonl \
  --max-tokens "$(jq .recommended_max_tokens /tmp/mix-report.json)" \
  --ignore-eos \
  --warmup-requests 2 \
  --sweep 1,2,4,8 --requests-per-stage 32 \
  --html report.html --csv requests.csv
```

`--prompts` requires `--max-tokens`. Without `--max-tokens` on a plain
`--prompt` chat sweep, the CLI prints a warning: output length is uncontrolled
and token throughput is not comparable across configs. Stage `config` stamps
`max_tokens`, `ignore_eos`, `min_tokens`, `prompts`, `prompt_pool_size`, and
`warmup_requests`.

The report plots achieved
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
With `--streaming`, `ttft=` and `tpot=` use captured stream timings (ITL is
retained on each CSV row; TPOT is `(service_latency - ttft) / (output_tokens - 1)`).
`user_tps=` is a minimum output tok/s per in-flight request
(`output_tokens / service_latency_s`); each sweep point reports `user_tps`
distributions plus `users_at_slo` (`load * meeting_fraction`) when that SLO is set.

Optional `--price-per-hour` (or `sut.cost.price_per_hour`) stamps
`cost_per_million_output_tokens` on each stage when output tok/s is known.

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

## Prometheus telemetry NDJSON

For durable hardware and engine time series during a sweep, pass `--ndjson`
with `--telemetry` (YAML Prometheus sources) or legacy `--metrics-url`. Optional
`--require-telemetry` aborts after consecutive scrape failures. The default
smoke source is the Metrum [all-smi](https://github.com/chetan-metrum-ai/all-smi)
fork at `http://127.0.0.1:9090/metric`. Scope, units, join model, and recipes:
[TELEMETRY.md](TELEMETRY.md). Analysis formulas: [telemetry/ANALYSIS.md](telemetry/ANALYSIS.md).

## OpenTelemetry

OTLP export is opt-in at build and runtime:

```bash
cargo build --release --features otlp
OTEL_EXPORTER_OTLP_HEADERS='authorization=Bearer token' \
metrum-ai-bench-cli-strategic ... --otlp-endpoint https://collector.example.com
```

The exporter emits standard OTLP/HTTP JSON spans to `/v1/traces` and summary
request counters plus p95 latency to `/v1/metrics`. Spans cover intended
schedule through completion and carry queue and service latency attributes.
No network telemetry occurs unless `--otlp-endpoint` is supplied.

## Mock server

`metrum-ai-bench-cli-mock-server` is a deterministic Rust fixture supporting health,
Prometheus metrics, chat/completions, embeddings, reranking, tool calls and
JSON-schema-shaped output. `--latency-ms` controls delay and `--fail-every N`
injects reproducible HTTP 503 responses. The existing Go dummy server remains
the deeper compatibility fixture.

It is shipped as a binary target in the published crate:

```bash
cargo install metrum-ai-bench-cli --bin metrum-ai-bench-cli-mock-server
```

## Distribution

The crate is publishable with `cargo publish`. Tagged releases cross-build Linux
gnu (glibc, not musl) and macOS Darwin archives with a pinned
`cargo-zigbuild` container on Linux, generate CycloneDX SBOMs, checksums and
keyless Sigstore signatures, smoke-test each archive on a matching native
runner, then create the GitHub Release. The crates.io upload runs only when
`CRATES_IO_PUBLISH=true` and `CRATES_IO_TOKEN` is set; it is skipped with a
warning otherwise, so the release itself still succeeds. Manual dispatch
requires a version-matching release tag and has a separate crates.io
publication switch.
