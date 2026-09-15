<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# OSS rollout campaign

Live GPU work happens **once**, after local fmt/clippy/tests/release
verification are green. Until then every `campaign.sh` command is dry-run
unless `--execute` is passed.

## Parallel instances

Launch **one retained Shadeform VM per lane**, not a create/destroy loop:

| Lane | Engine | Model | Endpoint |
|------|--------|-------|----------|
| `llm` | vLLM | `Qwen/Qwen2.5-7B-Instruct` | `/v1/chat/completions` |
| `vlm` | vLLM | `Qwen/Qwen2.5-VL-7B-Instruct` | `/v1/chat/completions` |
| `asr` | vLLM (Whisper-class, if the image serves `/v1/audio/transcriptions`) else dummy-certified | small ASR | `/v1/audio/transcriptions` |
| `imagegen` | OpenAI-compatible image server if a compact image is available; otherwise dummy-certified and labeled as such in `manifest.json` | small image model | `/v1/images/generations` |

GPU pick order is unchanged: RTX 6000 Pro Blackwell Server Edition → B200 →
H200 → H100 → L40S. Consumer SKUs are skipped. Instances stay up until the
campaign bundle is backed up and the case study is written.

## Sweep matrix (retained)

Each cell writes its own subdirectory under gitignored `live-results/`:

```
live-results/campaign-<id>/
  manifest.json
  instances.json
  <lane>/c<concurrency>-n<requests>/
    command.txt
    environment.json
    results.jsonl
    summary.json
    sha256sums
```

Default cells (override with env vars):

- **LLM:** concurrency `1,2,4,8`; closed-loop; then open-loop `--request-rate 4,8,16` with `--arrival constant` at concurrency cap 8. `--warmup-requests 8`, `--num-requests 64`, `--max-tokens 128`, `--seed 7`.
- **VLM:** concurrency `1,2,4`; streaming; same seed/warmup; one image per prompt.
- **ASR:** concurrency `1,2,4`; `--normalizer whisper-english`; ground truth present.
- **Imagegen:** concurrency `1,2`; `64x64` or the smallest practical size.

Every request record must include `schema_version`. Summaries must include
`n`, type-7 percentiles, and `partial: false` except for documented interrupts.

## Backup

After validation, snapshot `live-results/campaign-<id>/` with restic using
`env.json` (`RESTIC_REPO_HOST`, `RESTIC_REPO_PATH`, `RESTIC_PASSWORD`). Never
commit the snapshot ID into git; record it in the private campaign
`manifest.json` only. Restore to a temp dir and `diff -rq` before teardown.

## Case study

The public write-up uses **aggregates and plots**, not raw prompts if they
are proprietary. Compelling artifacts:

- LLM knee: goodput vs concurrency / offered rate
- TTFT and ITL p50/p95 vs concurrency
- VLM vs LLM latency at matched concurrency
- ASR RTFx and WER
- Imagegen images/s and latency

Reproducible demo commands live in `scripts/live/campaign.sh demo`.
