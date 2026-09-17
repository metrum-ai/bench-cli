<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Metrum AI Bench

[![CI](https://github.com/metrum-ai/bench-cli/actions/workflows/ci.yml/badge.svg)](https://github.com/metrum-ai/bench-cli/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)
[![Release](https://img.shields.io/github/v/release/metrum-ai/bench-cli)](https://github.com/metrum-ai/bench-cli/releases)

Apache-2.0 licensed load and performance measurement for OpenAI-compatible
LLM, VLM, ASR, and image-generation endpoints. Current release: **1.0.0-rc.6**
([CHANGELOG](CHANGELOG.md)).

Metrum AI Bench measures one environment and produces a result with a
manifest. Metrum AI Bench Platform (commercial) remembers, compares, governs,
and attests.

Large prompt corpora for LLM workload mixes are published on Hugging Face as
[`metrum-ai/prompt-library`](https://huggingface.co/datasets/metrum-ai/prompt-library).
Use `metrum-ai-bench-prompts` to select a mix by ISL/OSL mean or median within
CLI tolerances, then feed the resulting JSONL to `metrum-ai-bench-llm` (see
[docs/PROMPT_LIBRARY.md](docs/PROMPT_LIBRARY.md)). This repository ships only
tiny fixtures ([test-data/README.md](test-data/README.md)). Draft card for
other modality fixtures:
[docs/datasets/DATASET_CARD.draft.md](docs/datasets/DATASET_CARD.draft.md).

## Quickstart (60 seconds)

One publishable LLM run against the local dummy server. Needs Go 1.26.6+ for
the dummy server; the benchmark binaries themselves do not.

```bash
# 1. Build (or unpack a release tarball; see Install)
cargo build --release

# 2. Start a local OpenAI-compatible server that needs no credentials (needs Go 1.26.6+)
go run ./dummy-model-server/cmd/dummy-model-server -port 18321 -latency 100ms -chunk-interval 20ms &

# 3. Describe what you are testing. Copy and edit examples/sut.example.json.
cp examples/sut.example.json sut.json

# 4. Run. --api-key is always required with --url; the dummy server accepts any value.
target/release/metrum-ai-bench llm -- \
  --url http://127.0.0.1:18321/v1/chat/completions --api-key dummy \
  --scenario quickstart --model dummy --mode chat --streaming \
  --prompts test-data/llm-hi.jsonl \
  --num-requests 16 --concurrency 4 --max-tokens 64 --seed 7 \
  --sut sut.json --require-sut \
  --data-log results.jsonl

# 5. Read the result. The last line of results.jsonl is the run summary.
tail -n 1 results.jsonl | jq '{requests: .attempted, errors: .errors, ttft_p50_s: .ttft_s.p50, tokens_per_s: .completion_tokens_per_second, sut: .sut}'
```

`--sut` is the operator-declared system under test embedded in the summary.
`--require-sut` refuses to run without that block and is what makes the number
publishable; see [Publishing a result](#publishing-a-result). If you are about
to test a thinking model, read [Reasoning models](#reasoning-models) first.

## Install

**GitHub Releases** (preferred for binaries): download the tarball for your
target from [Releases](https://github.com/metrum-ai/bench-cli/releases), verify
the `.sha256` and optional Sigstore bundle, then unpack. Release archives are
cross-built with [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild)
on Linux for `x86_64`/`aarch64` **glibc** (`*-unknown-linux-gnu`, glibc 2.17
floor) and macOS Darwin, not musl. TLS is rustls (no OpenSSL link).

**From source:**

```bash
git clone https://github.com/metrum-ai/bench-cli.git
cd bench-cli
cargo build --release
```

Rust 1.85 or later is required. The dummy server used in Quickstart and the
examples requires Go; the benchmark binaries do not. Pass `--quiet` or set
`NO_BANNER=1` to suppress ASCII banner art (a one-line identity still prints).
Optional Homebrew formula is attached to each GitHub Release
(`metrum-ai-bench.rb`); a tap publish runs when the release workflow is
configured with a Homebrew tap repository.

```bash
cargo test --all-targets
```

## Tools

| Entry | Measures | When to use |
|-------|----------|-------------|
| `metrum-ai-bench llm` / `metrum-ai-bench-llm` | Chat/completion latency, TTFT, ITL/TPOT, token throughput | Text OpenAI-compatible `/v1/chat/completions` or completions |
| `metrum-ai-bench vlm` / `metrum-ai-bench-vlm` | Same as LLM plus image payload size | Vision models with `image_url` / `image_urls` prompts |
| `metrum-ai-bench asr` / `metrum-ai-bench-asr` | Transcription latency, RTFx, optional WER/CER | `/v1/audio/transcriptions` |
| `metrum-ai-bench imagegen` / `metrum-ai-bench-imagegen` | Image generation latency and artifact hashes | `/v1/images/generations` |
| `metrum-ai-bench prompts` / `metrum-ai-bench-prompts` | ISL/OSL mix selection from `metrum-ai/prompt-library` | Build a JSONL prompt set with target mean/median lengths |
| `metrum-ai-bench selftest` | Local sanity check of the install | After build or release unpack |
| `metrum-ai-bench-strategic` | Concurrency/rate sweeps, knee, sessions, exports | Capacity planning and multi-turn validity (separate binary, not a unified subcommand) |
| `metrum-ai-bench-mock-server` | Deterministic OpenAI-compatible mock for strategic fixtures | Local strategic tests without the Go dummy |

The preferred entry point for modalities is `metrum-ai-bench` with those
subcommands. During the v1.x compatibility period the four modality binaries
can also be invoked directly; deprecated `metrumbench-*` shims remain (they
print a v2.0 removal notice). `metrum-ai-bench-strategic` is a separate
binary. Shared load flags live in clap common args; modality-specific flags
are in [docs/CLI.md](docs/CLI.md) (regenerated from `--help`).

## Reasoning models

Thinking models emit reasoning deltas before the answer. A small
`--max-tokens` often yields `no_output_token` on every request, and a naive
TTFT that counts reasoning looks falsely fast. Read
[docs/REASONING_MODELS.md](docs/REASONING_MODELS.md) before choosing
`--max-tokens` or `reasoning_effort`.

## Publishing a result

A published number that names Metrum AI Bench must carry an unmodified run
summary with a SUT block. Produce a compliant run with:

```bash
metrum-ai-bench llm -- \
  --url http://127.0.0.1:18321/v1/chat/completions --api-key dummy \
  --scenario publish --model dummy --mode chat --streaming \
  --prompts test-data/llm-hi.jsonl \
  --num-requests 16 --concurrency 4 --max-tokens 64 \
  --data-log results.jsonl \
  --sut sut.json --require-sut
```

Example SUT declaration (`examples/sut.example.json`). Every field except the
defaulted `provenance` is optional; omit or null what you do not know:

```json
{
  "provenance": "declared",
  "name": "example-host / GPU x1",
  "vendor": "Example OEM",
  "gpu": { "model": "L40S", "count": 1, "memory_gb": 48 },
  "cpu": "Example CPU",
  "memory_gb": 256,
  "driver_version": "NVIDIA 580.xx",
  "runtime": { "name": "vllm", "version": "latest", "config": "TP=1" },
  "model": { "id": "example/model", "revision": null, "quantization": null },
  "host_os": "Ubuntu 22.04",
  "notes": "Example only: replace with your declared inventory."
}
```

`--require-sut` implies `--redact-hostname`. Every closing-card or blog number
must trace to a `summary.json` (or the closing `metrum-ai-bench.summary.v3`
line in a published results directory); see
[docs/RESULTS_PUBLICATION_POLICY.md](docs/RESULTS_PUBLICATION_POLICY.md).

A claim that omits the manifest (including the SUT block) is **not** a
Metrum AI Bench result under the publication policy, even if the software was
used. See also [TRADEMARKS.md](TRADEMARKS.md).

## Input formats

Prompt/input files are **JSONL only** (`.csv` is rejected with a migration
hint). Examples:

**LLM**

```json
{"prompt":"Write a haiku about latency."}
{"prompt":"Summarize coordinated omission in one sentence."}
```

**VLM**

```json
{"prompt":"Describe the image.","image_urls":["https://example.com/a.png"]}
{"prompt":"Count the objects.","image_url":"test-data/tiny.png"}
```

**ASR**

```json
{"id":"sample-1","path":"test-data/dummy.mp3","format":"mp3","duration":2.0}
{"id":"sample-2","url":"https://example.com/clip.wav","format":"wav","duration":1.5}
```

Optional ASR ground truth is JSONL with matching `id` and `transcript`.
WER/CER use `--normalizer` (`whisper-english` default, `whisper-basic`, or
`none`); the choice is recorded in `config.normalizer`. See
[docs/ASR.md](docs/ASR.md) for more detail.

**Imagegen:** pass `--prompt` once, or `--prompts` JSONL:

```json
{"id":"p0","prompt":"a red cube on a table"}
{"prompt":"a blue sphere","negative_prompt":"blurry","size":"512x512"}
```

## Prompt library

Success is ISL/OSL **within tolerance**, not an exact `--count`. After extract,
use `report.selected_count` and `report.recommended_max_tokens`. Keep
`--warmup-requests 0` so llm does not drop measured mix slots. llm still uses
one global `--max-tokens` (per-request caps are out of scope). Details:
[docs/PROMPT_LIBRARY.md](docs/PROMPT_LIBRARY.md).

```bash
cargo build --release --bin metrum-ai-bench-prompts --bin metrum-ai-bench-llm
go run ./dummy-model-server/cmd/dummy-model-server \
  -port 18321 -latency 100ms -chunk-interval 20ms

# Median targets (sample config; pin a commit SHA)
target/release/metrum-ai-bench-prompts \
  --revision 0666f62e581b482838ae2e17b333ee36ff3d01b0 --config sample \
  --count 64 --seed 42 \
  --isl-target 512 --isl-unit tokens --isl-stat median --isl-tolerance 64 \
  --osl-target 128 --osl-unit tokens --osl-stat median --osl-tolerance 32 \
  --output /tmp/mix.jsonl --report /tmp/mix-report.json

target/release/metrum-ai-bench-llm \
  --url http://127.0.0.1:18321/v1/chat/completions --api-key dummy \
  --scenario prompt-library-median \
  --num-requests "$(jq .selected_count /tmp/mix-report.json)" \
  --concurrency 4 --warmup-requests 0 --prompts /tmp/mix.jsonl \
  --mode chat --streaming --model dummy \
  --max-tokens "$(jq .recommended_max_tokens /tmp/mix-report.json)" \
  --seed 42 --data-log /tmp/mix-run.jsonl
```

Mean-target extract (same llm/dummy pattern afterward):

```bash
target/release/metrum-ai-bench-prompts \
  --revision 0666f62e581b482838ae2e17b333ee36ff3d01b0 --config sample \
  --count 32 --seed 7 \
  --isl-target 256 --isl-unit tokens --isl-stat mean --isl-tolerance 32 \
  --osl-target 128 --osl-unit tokens --osl-stat mean --osl-tolerance 16 \
  --output /tmp/mix-mean.jsonl --report /tmp/mix-mean-report.json
```

```bash
target/release/metrum-ai-bench selftest
target/release/metrum-ai-bench llm -- \
  --url http://127.0.0.1:8000/v1/chat/completions \
  --api-key dummy --scenario example --num-requests 100 --concurrency 8 \
  --prompts prompts.jsonl --mode chat --streaming --model example \
  --max-tokens 64 --data-log results.jsonl --seed 7 \
  --warmup-requests 8 --request-rate 20 --arrival poisson \
  --max-concurrency 64 --slo ttft=250ms --slo e2e=2s
```

Pass `--runs N` among the forwarded modality arguments to the unified entry
point to execute sequential independent runs and append a seeded bootstrap
cross-run aggregate to `--data-log`.

The `metrum-ai-bench-strategic` runner adds concurrency/rate sweeps
(`--sweep`), knee detection, multi-turn sessions, validity rules,
server-metrics correlation, and CSV, HTML (`--html`), MLPerf-shaped
(`--mlperf-dir`), and optional OTLP exports. See
[strategic benchmarking](docs/STRATEGIC_BENCHMARKING.md).

## Dummy server

```bash
go run ./dummy-model-server/cmd/dummy-model-server \
  -port 18321 -latency 100ms -chunk-interval 20ms
```

Then run any modality against `http://127.0.0.1:18321` with `--api-key dummy`.
See [docs/REPRODUCING.md](docs/REPRODUCING.md) for the checked-in LLM reference
and `dummy-model-server/README.md` for flags covering VLM/ASR/imagegen.

For deterministic strategic fixtures, the Rust mock server binary is
`metrum-ai-bench-mock-server` (see [strategic benchmarking](docs/STRATEGIC_BENCHMARKING.md)).

Compact VLM / ASR / imagegen examples (second shell, after the dummy is up):

```bash
printf '%s\n' '{"prompt":"Hi","image_url":"test-data/tiny.png"}' > /tmp/vlm.jsonl
target/release/metrum-ai-bench-vlm --url http://127.0.0.1:18321/v1/chat/completions \
  --api-key dummy --scenario vlm --num-requests 4 --concurrency 2 \
  --prompts /tmp/vlm.jsonl --model dummy --max-tokens 16 \
  --data-log /tmp/vlm.jsonl.out --streaming

printf '%s\n' '{"id":"a","path":"test-data/dummy.mp3","format":"mp3","duration":2.0}' > /tmp/asr.jsonl
target/release/metrum-ai-bench-asr --url http://127.0.0.1:18321/v1/audio/transcriptions \
  --api-key dummy --scenario asr --num-requests 4 --concurrency 2 \
  --input /tmp/asr.jsonl --model dummy --data-log /tmp/asr.jsonl.out

target/release/metrum-ai-bench-imagegen --url http://127.0.0.1:18321/v1 \
  --api-key dummy --scenario img --num-requests 2 --concurrency 1 \
  --prompt "a square" --model dummy --size 64x64 --data-log /tmp/img.jsonl
```

Every completed request is flushed incrementally to JSONL. A graceful Ctrl-C
stops issuance, drains already-started work, and emits a `partial: true`
summary. Warmup records remain auditable but are excluded from measured
distributions.

## Metrics (summary)

| Metric | Meaning |
|--------|---------|
| E2E latency | Send → body complete (`latency_s`) |
| TTFT | First visible token (`ttft_s`); not first-byte headers |
| First byte | Headers received (`first_byte_s`) |
| Window rps | Measured successes / monotonic send→complete window |
| Goodput | Successes meeting `--slo` thresholds / window |
| RTFx (ASR) | Audio seconds / client request seconds |

Full definitions: [docs/METRICS.md](docs/METRICS.md). Also see
[output schema](docs/OUTPUT_SCHEMA.md), [CLI reference](docs/CLI.md),
[prompt library](docs/PROMPT_LIBRARY.md), [reproduction](docs/REPRODUCING.md),
[comparison notes](docs/COMPARISON.md),
[reasoning models](docs/REASONING_MODELS.md), and
[known limitations](docs/LIMITATIONS.md).

## Security and provenance

Do not put credentials in endpoint files committed to source control. Secret
scanning runs in CI. Test fixture provenance is documented in
`test-data/README.md`; dependencies and notices are in
`THIRD_PARTY_LICENSES` and `NOTICE`.

Run summaries may record client `environment.hostname`. Use
`--redact-hostname` when publishing or sharing logs so the hostname field is
null; `--require-sut` implies redaction for publication-oriented runs. See
[docs/OUTPUT_SCHEMA.md](docs/OUTPUT_SCHEMA.md) for the summary / environment
shape.

## Known limitations

Scope and honesty constraints (client-side only, SUT declaration, NVIDIA-only
smoke matrix, unofficial MLPerf export, and more):
[docs/LIMITATIONS.md](docs/LIMITATIONS.md).

## License

Apache License 2.0. See `LICENSE`.

Metrum AI and Metrum AI Bench are trademarks of Metrum AI, Inc. See
[TRADEMARKS.md](TRADEMARKS.md).
