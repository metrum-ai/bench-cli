<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# LLM bake-off: Metrum AI Bench CLI vs AIPerf

Controlled, on-host comparison of **metrum-ai-bench-cli** and
[ai-dynamo/aiperf](https://github.com/ai-dynamo/aiperf) against the same vLLM
endpoint. Load generators ran on the GPU host (loopback), not from a laptop.

Artifact bundle: [bakeoff/20260923-pro6000/](bakeoff/20260923-pro6000/).

## Setup

| Field | Value |
|-------|--------|
| Host | Shadeform RTX PRO 6000 Blackwell Server Edition (SSH `216.243.220.66`) |
| Server | Vendor vLLM Docker on host, OpenAI-compatible at `http://127.0.0.1/v1` |
| Model | `Qwen/Qwen2.5-7B-Instruct` |
| Metrum CLI | `1.2.0` release binaries built on host |
| AIPerf | `aiperf` 0.11.x in a fresh venv (`pip install aiperf`) |
| Workload | Hugging Face `metrum-ai/prompt-library` rev `0666f62e581b482838ae2e17b333ee36ff3d01b0`, profile `chat-short`, `--config full`, count 32, seed 42 |
| Targets | ISL median 256 tokens (achieved 256), recommended `max_tokens` / OSL mean 48 |
| Sweep | concurrency / load `1,2,4,8,16`; 32 requests per stage; Metrum warmup 4; AIPerf `--warmup-request-count 4` |
| SUT | `bakeoff/20260923-pro6000/sut.json` (declared provenance, on-host loopback) |

Matched AIPerf run used the same prompt texts converted to AIPerf
`single_turn` JSONL (`{"text": ...}`) via `--input-file` /
`--custom-dataset-type single_turn`. An earlier exploratory AIPerf run with
synthetic ISL≈550 is **not** used for the comparison table.

## Session capture (UX)

Codex CLI (`0.156.1`) on the GPU host, driven by Kimi Open Platform
(`kimi-k2.7-code`) through a local `@codeproxy/cli` Responses bridge on
`:8787`. Sessions recorded with `asciinema` plus `script` transcripts:

- `bakeoff/20260923-pro6000/codex/codex-metrum.cast`
- `bakeoff/20260923-pro6000/codex/codex-aiperf.cast`
- Matching `.typescript` transcripts alongside the casts

Automation path also recorded with `script` as
`bakeoff/20260923-pro6000/bakeoff-session.typescript`.

Operator prompts used on host:
`scripts/live/bakeoff/CODEX_PROMPT_METRUM.md`,
`CODEX_PROMPT_AIPERF.md`, `CODEX_KIMI.md`.

## Phase timings (wall clock)

From `bakeoff/20260923-pro6000/timings.jsonl` (UTC):

| Phase | Tool | Duration (s) |
|-------|------|-------------:|
| host_prep | shared | 0 |
| install_metrum | metrum | 0 (already on PATH) |
| install_aiperf | aiperf | 25 |
| prompt_extract | metrum | 2 |
| sut_write | shared | 0 |
| metrum_sweep | metrum | 33 |
| aiperf_sweep (synthetic, exploratory) | aiperf | 10 |
| aiperf_matched_sweep (prompt-library) | aiperf | 64 |
| client_ceiling (mock) | metrum | 1 |

Takeaway: AIPerf Python venv install dominates first-run setup; matched sweep
wall time is longer than Metrum for the same concurrency ladder because AIPerf
runs each concurrency as a separate profile variation with tokenizer/setup
overhead.

## Throughput comparison (matched prompts)

Output completion tokens per second (aggregate), zero errors on both tools:

| Concurrency | Metrum tok/s | AIPerf tok/s | Delta (AIPerf − Metrum) |
|------------:|-------------:|-------------:|------------------------:|
| 1 | 85.95 | 85.03 | −0.9 (−1.1%) |
| 2 | 169.13 | 167.22 | −1.9 (−1.1%) |
| 4 | 340.26 | 325.46 | −14.8 (−4.3%) |
| 8 | 586.80 | 626.04 | +39.2 (+6.7%) |
| 16 | 1001.17 | 1133.25 | +132.1 (+13.2%) |

Notes:

- AIPerf ISL p50 was **256.50** across stages (matches prompt-library target).
- AIPerf TTFT p50 stayed ~22–35 ms; Metrum reports rich connect / prefill /
  decode / observed-concurrency distributions (see `metrum-summary.json`).
- Metrum knee selected load **8** at 586.80 tok/s (knee heuristic, not a claim
  of saturation).
- At higher concurrency, absolute tok/s diverge within ~13%. Causes are not
  isolated here (request cadence, tokenizer/usage accounting, warmup handling,
  connection reuse). Numbers are close enough to treat as same-order results
  on this SUT.

Sources: `metrum-summary.json` points; AIPerf
`aiperf/concurrency_*/profile_export_aiperf.csv`.

## UX and product observations

| Topic | Metrum | AIPerf |
|-------|--------|--------|
| Install | Single static binary (or cargo build) | Python venv + pip (~25 s cold) |
| Prompt library | Native `metrum-ai-bench-cli-prompts` with profiles, revision pin, mix report | Needs JSONL conversion to `single_turn` / synthetic ISL/OSL |
| Publishable stamp | `--sut` / `--require-sut` on strategic | Strong artifact dirs; no equivalent SUT stamp |
| Sweep UX | One CLI invocation, knee + HTML/CSV | `--concurrency 1,2,4,...` multi-run with rich CSV/JSON per cell |
| Diagnostics | Observed concurrency, connect/prefill/decode, SLOs, cost/M | Broad HTTP/GPU/server metric catalog; DCGM optional |
| Agent path | Explicit flags; Codex drove summary cleanly | Same; more flags to discover for matched workloads |

## Client ceiling

Mock-server high-concurrency probe completed in ~1 s on host (see
`ceiling-summary.json` on the host run directory). It bounds client capacity
separate from the GPU SUT; not used as a GPU score.

## Go / no-go

**Go for using Metrum AI Bench CLI for publishable on-host LLM load studies**
when you need SUT stamps, prompt-library mixes, strategic knee/HTML, and
observed-concurrency diagnostics.

**Keep AIPerf** when you need NVIDIA-oriented telemetry breadth, multi-modality
beyond Metrum's four OpenAI-compatible clients, or ecosystem familiarity.

Neither tool "wins" absolute tok/s on this single short chat-short ladder;
agreement within ~13% at matched ISL/OSL is the useful finding. Prefer
loopback / on-host loadgen for TTFT and concurrency claims.

## Reproduce

On the GPU host (not from a laptop):

```bash
# After metrum release bins and scripts/live/bakeoff are present:
export BENCH_URL=http://127.0.0.1/v1/chat/completions
export BENCH_MODEL=Qwen/Qwen2.5-7B-Instruct
./scripts/live/bakeoff/run_on_host.sh
# Optional Codex UX (see CODEX_KIMI.md):
# asciinema rec -c 'codex' codex-metrum.cast
```

Matched AIPerf flags used for the table:

```bash
aiperf profile --model Qwen/Qwen2.5-7B-Instruct \
  --url http://127.0.0.1 --endpoint-type chat --streaming \
  --concurrency 1,2,4,8,16 --request-count 32 --warmup-request-count 4 \
  --output-tokens-mean 48 \
  --input-file aiperf-prompts.jsonl --custom-dataset-type single_turn \
  --artifact-dir aiperf-artifacts
```

Closes GitHub issue #144 (epic #148).
