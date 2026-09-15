<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Metrum AI Bench

Apache-2.0 licensed load and performance measurement for OpenAI-compatible
LLM, VLM, ASR, and image-generation endpoints.

## Build

Rust 1.85 or later is required.

```bash
cargo build --release
cargo test --all-targets
```

The preferred entry point is `metrum-ai-bench` with `llm`, `vlm`, `asr`,
`imagegen`, and `selftest` subcommands. During the compatibility period the
four modality binaries can also be invoked directly. Run a command with
`--help` for its authoritative flags.

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

The `metrum-ai-bench-strategic` runner adds concurrency/rate sweeps, knee
detection, multi-turn sessions, validity rules, server-metrics correlation,
and CSV, HTML, MLPerf, and optional OTLP exports. See
[strategic benchmarking](docs/STRATEGIC_BENCHMARKING.md).

Prompt files are JSONL with a `prompt` string. VLM rows additionally use
`image_urls` (array) or `image_url` (string). ASR input rows contain `id`,
`path` or `url`, `format`, and optional `duration`.

Every completed request is flushed incrementally to JSONL. A graceful Ctrl-C
stops issuance, drains already-started work, and emits a `partial: true`
summary. Warmup records remain auditable but are excluded from measured
distributions.

## Methodology

- Closed-loop concurrency or seeded constant/Poisson open-loop arrivals.
- Open-loop latency includes queue delay from scheduled arrival, preventing
  coordinated omission from disappearing from headline latency.
- Hyndman-Fan type 7 percentiles, sample count, sample standard deviation,
  MAD, p99 reliability, throughput bins, and seeded bootstrap helpers.
- TTFT is the first visible token; reasoning time is separate; ITL records
  visible-token chunk intervals; TPOT divides decode duration by `N - 1`.
- Per-endpoint distributions are first class and aggregate distributions are
  marked as mixtures.
- Optional local counts from `tokenizer.json`:
  `cargo build --features tokenizer`.
- SLO goodput uses repeatable `--slo ttft=`, `tpot=`, and `e2e=` thresholds.

See [metric definitions](docs/METRICS.md), [output schema](docs/OUTPUT_SCHEMA.md),
[reproduction procedure](docs/REPRODUCING.md), and
[comparison notes](docs/COMPARISON.md).

## Security and provenance

Do not put credentials in endpoint files committed to source control. Secret
scanning runs in CI. Test fixture provenance is documented in
`test-data/README.md`; dependencies and notices are in
`THIRD_PARTY_LICENSES` and `NOTICE`.

## License

Apache License 2.0. See `LICENSE`.
