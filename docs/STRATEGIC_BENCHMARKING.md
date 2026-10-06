<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Strategic benchmarking

**Search first.** Before a sweep against a real server, web-search the
current vendor docs for the model and engine version. Check the launch
arguments (max sequences, context length, memory), the sampling and thinking
parameters, and the concurrency range the engine supports, then pick
`--sweep` and the request counts from that. Record sources in the SUT. The
engine map is in [SERVING.md](SERVING.md).

For agent-bench, start from the streaming LLM/chat path so TTFT measures visible
output as it arrives. The unary strategic path cannot measure TTFT. Strategic
chat turns opt in with `--streaming`, including `--sessions`; their per-turn CSV
records include `first_byte_s` and `ttft_s`, and measured TTFT enforces `--slo ttft=`.
Streaming is off by default and applies to chat and vlm. Embeddings, rerank,
asr and imagegen remain unary, and `--tools` cannot be combined with
`--streaming`.


`metrum-ai-bench-cli-strategic` is the runner for concurrency/rate sweeps, chat
sessions, structured output, embeddings, reranking, VLM, ASR and image
generation sweeps, server correlation and portable exports. Existing modality-specific binaries remain supported.

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
`n`, latency percentiles, throughput, goodput, and knee detection. Stage
throughput, goodput, and token rates divide by the stage window: earliest
measured send (any outcome) to the latest successful completion, or the
latest completion of any outcome when the stage has no success (#224).
Measured prompt indexing restarts at zero after warmup so the mix is not
shifted.
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

Knee detection needs at least 5 measured stages (stages with a p95): both
endpoints plus 3 interior candidates. Stages with no successes have no p95
and do not count. With 3 stages Kneedle has a
single interior candidate and always returns the middle stage, so shorter
sweeps report no knee instead of a misleading one. Plan sweeps with 5 or more
loads (for example `--sweep 1,2,4,8,16,32`) when the knee matters. When there
is no knee, stdout JSON has `"knee": null` and `knee_detection.reason` says
why:

- `insufficient_points`: fewer than `knee_detection.min_points` (5) measured
  stages (stages with a p95).
- `missing_latency`: the first or last stage has no p95 (no successes), and
  neither a p95 bend nor a saturated stage gives a knee.
- `flat_curve`: throughput or p95 does not change across the measured
  stages, or p95 bends over flat throughput.
- `no_bend`: p95 rises less than 20% above its running minimum and no stage
  is saturated, so the curve has no meaningful bend (#232).

A knee has two candidates, and the earlier stage wins
(`knee_detection.method` says which):

- `kneedle`: the p95 rise is the largest rise over the running minimum,
  `max_j (p95_j / min_{i<=j} p95_i - 1)` (`knee_detection.p95_rise`). When it
  is at least 20%, Kneedle runs on the measured stages from that baseline
  `i` to that peak `j`, normalized by the data range. An inflated cold first
  stage, a mid-sweep bend that recovers by the last stage, or a last stage
  that drops below the first cannot hide the bend. A p95 that only falls has
  no rise.
- `saturation`: the stage before the first saturated stage
  (`knee_detection.saturated_index`). On a concurrency sweep a stage is
  saturated when its relative throughput gain is under half its relative
  load gain, `(X_i - X_{i-1}) / X_{i-1} < 0.5 * (load_i - load_{i-1}) /
  load_{i-1}` (`X` is success throughput; on 2x steps, a throughput ratio
  below 1.5). Stages that both run all requests in one wave
  (`load >= requests_per_stage`) are skipped. On any sweep a stage is also
  saturated when its error rate is 5 points or more above the lowest earlier
  stage. Rate sweeps use only the error rate: their stage window ends at the
  latest completion (#224), so latency spread, not saturation, moves
  achieved throughput. Load shedding (fast 429/503, admission control) keeps
  success p95 flat while throughput plateaus or failures rise, so p95 alone
  misses it. Earlier wins, so shedding ahead of a later p95 bend is reported
  where it starts, and a later all-failure stage does not override an
  earlier bend.

Kneedle always returns the interior stage farthest from the chord, so a nearly
linear sweep would still report a knee. The 20% minimum p95 rise
(`KNEE_MIN_P95_RISE` in `src/strategic.rs`) is provisional: it is set from two
live curves. It separates the live H100 sweeps from the #184 validation (vLLM
0.31.0). The Qwen3-VL-8B sweep at c=1..16 scaled throughput almost linearly
(0.94 to 12.37 req/s) while p95 rose only 11% (1.200 to 1.333 s), and each 2x
step gained 1.87x to 1.97x throughput, so it reports `no_bend`. A
fixed-latency mock sweep also reports `no_bend` (the analyze.py
`sweep5_no_bend` fixture). The `sweep5` fixture puts a 4-request capacity gate
in front of the same mock, so p95 stays at 0.203 to 0.204 s up to c=4, then
reaches 0.409 s at c=8 and 1.419 s at c=16, and the knee is c=4 (#240). The
LLM sweep at c=1..64 rose 41% (0.632 to 0.892 s) and keeps its knee at c=32.
The LLM sweep's smallest step gain is 1.77x. A threshold on the normalized
chord distance cannot make this split: the LLM curve peaks at 0.095, below the
VLM curve's 0.164. At 20%, the threshold sits about 2x above the bend-free
rises and about 2x below the smallest real bend. A sweep with less than 20%
p95 rise usually has not reached saturation; check the error rate and
throughput scaling per stage, then extend the sweep to higher loads to find
the knee.

`knee_detection` is always present:
`{"index", "reason", "points", "min_points", "method", "p95_rise", "saturated_index", "min_p95_rise", "min_marginal_gain", "max_error_rate_rise"}`.
See [OUTPUT_SCHEMA.md](OUTPUT_SCHEMA.md) for each field.
`reason` is null exactly when `index` is set. The CLI also prints
`note: no knee: ...` on stderr (only for runs with 2 or more stages, so
single-stage and sessions runs stay quiet) and the HTML report shows the same sentence.
Analysis that keys on the knee (for example `kv_cache_util_at_knee`) should
report null, not 0, when `knee` is null.
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

## VLM, ASR and image generation sweeps

`--kind vlm`, `--kind asr` and `--kind imagegen` sweep the same endpoints as
the modality binaries and build requests with their shared request builders
(`src/vlm.rs`, `src/asr.rs`, `src/imagegen.rs`), so a sweep point and a
single modality run send the same bodies. `--url` is the full endpoint URL.
Inputs load once before the first request: images are read and base64
encoded, and audio is read (or downloaded once) into memory, so file I/O never
enters a measured latency.

```bash
# VLM: chat completions with image_url parts; --max-tokens is required.
metrum-ai-bench-cli-strategic --kind vlm \
  --url http://127.0.0.1:8000/v1/chat/completions --model MODEL --api-key dummy \
  --prompts vlm.jsonl --max-tokens 128 --streaming --sweep 1,2,4,8,16

# ASR: multipart uploads; --ground-truth adds stage WER/CER.
metrum-ai-bench-cli-strategic --kind asr \
  --url http://127.0.0.1:8000/v1/audio/transcriptions --model MODEL --api-key dummy \
  --audio-samples audio.jsonl --ground-truth refs.jsonl --sweep 1,2,4,8,16

# Image generation: b64_json images are decoded and digested.
metrum-ai-bench-cli-strategic --kind imagegen \
  --url http://127.0.0.1:8000/v1/images/generations --model MODEL --api-key dummy \
  --prompts prompts.jsonl --image-size 1024x1024 --sweep 1,2,4
```

| Kind | Inputs | Kind flags |
|------|--------|------------|
| `vlm` | `--prompts` VLM JSONL (as `metrum-ai-bench-cli-vlm --prompts`), or `--prompt` plus repeatable `--image` | `--max-tokens` (required), `--image-detail`, `--max-image-dimension`, `--temperature` (default 0.1), `--ignore-eos`, `--min-tokens`, `--extra-body-json`, `--streaming` |
| `asr` | `--audio-samples` JSONL (as `metrum-ai-bench-cli-asr --input`), optional `--ground-truth` | `--asr-response-format`, `--language`, `--normalizer` |
| `imagegen` | `--prompts` JSONL with `prompt`, or `--prompt` | `--image-size`, `--images-per-request`, `--image-response-format`, `--extra-body-json` |

Every sweep point keeps the usual stage summary (throughput, latency, HTTP
phases, time-weighted blocks, `knee_detection`) and adds `modality_metrics`:
one distribution per key over measured successes, with the key names the
modality binaries use on `request.v3` records. VLM: `image_count`,
`image_bytes`. ASR: `wer`, `cer` (with references), `rtfx_client` and
`audio_duration_s` (when the sample row has `duration`). Imagegen:
`images_requested`, `images_returned`, plus `image_digests` (`images`,
`distinct`) for decoded `b64_json` images. A key with no measured value is
`n = 0`, never a fabricated number. VLM also reports the chat TTFT and token
metrics; ASR and imagegen are unary, so their `ttft_s` is `n = 0`.
`config.modality` records the kind settings. For asr and imagegen, latency
ends when the response body is read; parsing, WER/CER and image decoding run
after it and never hold a concurrency slot. The ASR binary's latency also
includes form building and parsing, so sweep ASR latency can read slightly
lower (see `docs/METRICS.md`). WER/CER use the ASR binary's scorer, and
`rtfx_client` needs a positive sample `duration` in both. For vlm,
`--image` goes with `--prompt` (not `--prompts`) and `--shared-prefix` is
rejected. Telemetry (`--telemetry`,
`--ndjson`) works the same for every kind. Per-request modality values are
not written to the CSV, which keeps its columns for every kind.

## Prometheus telemetry NDJSON

For durable hardware and engine time series during a sweep, pass `--ndjson`
with `--telemetry` (YAML Prometheus sources) or legacy `--metrics-url`. Optional
`--require-telemetry` aborts after consecutive scrape failures. The default
smoke source is the Metrum [all-smi](https://github.com/chetan-metrum-ai/all-smi)
fork at `http://127.0.0.1:9090/metrics`, plus the serving engine's `/metrics`.
The series list is the YAML you pass, read at startup, not a catalog compiled
into the binary. Scope, units, join model, and recipes:
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
