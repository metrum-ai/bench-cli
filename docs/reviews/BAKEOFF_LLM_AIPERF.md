<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# LLM bake-off: Metrum AI Bench CLI vs AIPerf (actual vLLM runs)

Controlled, **on-host** comparison against a live **vLLM** OpenAI-compatible
server. Load generators ran on the GPU host (loopback), not from a laptop.

Primary artifact bundle:
[bakeoff/20260923-pro6000/](bakeoff/20260923-pro6000/).

## Executive summary

On Shadeform **RTX PRO 6000 Blackwell** serving `Qwen/Qwen2.5-7B-Instruct` via
vendor vLLM Docker, with a matched Hugging Face prompt-library mix
(`chat-short`, ISL median 256, `max_tokens` 48):

| Concurrency | Metrum tok/s | AIPerf tok/s | Delta % |
|------------:|-------------:|-------------:|--------:|
| 1 | 85.95 | 85.03 | −1.1% |
| 2 | 169.13 | 167.22 | −1.1% |
| 4 | 340.26 | 325.46 | −4.3% |
| 8 | 586.80 | 626.04 | +6.7% |
| 16 | 1001.17 | 1133.25 | +13.2% |

Zero errors on both tools. Low concurrency agrees within ~1–4%. Higher
concurrency diverges within ~13% (not isolated: cadence, early EOS / OSL,
tokenizer accounting). **Go** for Metrum on publishable on-host LLM studies
with SUT stamps and prompt-library profiles; **keep AIPerf** for NVIDIA
telemetry breadth.

## System under test (actual run)

| Field | Value |
|-------|--------|
| Date (UTC) | 2026-09-23 |
| GPU | NVIDIA RTX PRO 6000 Blackwell Server Edition × 1 |
| Host | Shadeform SSH `216.243.220.66` |
| Runtime | vLLM vendor Docker, OpenAI-compatible at `http://127.0.0.1/v1` (loopback :80) |
| Model | `Qwen/Qwen2.5-7B-Instruct` |
| Metrum | `metrum-ai-bench-cli-strategic` **1.2.0** (built on host) |
| AIPerf | `aiperf` **0.11.x** (`pip install aiperf` in host venv) |
| Workload | `metrum-ai/prompt-library` rev `0666f62e581b482838ae2e17b333ee36ff3d01b0`, profile `chat-short`, `--config full`, count 32, seed 42 |
| ISL | target median 256 (achieved 256); AIPerf measured ISL p50 **256.50** |
| OSL | `max_tokens` / `--output-tokens-mean` **48** |
| Sweep | concurrency / load `1,2,4,8,16`; 32 requests per stage; warmup 4 |
| SUT file | [sut.json](bakeoff/20260923-pro6000/sut.json) (declared provenance) |

### Serve config notes

Shadeform provided the vendor vLLM Docker endpoint on the GPU host. Qwen's
documented vLLM path is `vllm serve Qwen/Qwen2.5-7B-Instruct` (default OpenAI
port 8000; see [Qwen vLLM deploy](https://qwen.readthedocs.io/en/v2.5/deployment/vllm.html)
and the [Hub model card](https://huggingface.co/Qwen/Qwen2.5-7B-Instruct)).
This bake-off used the instance's exposed loopback chat-completions URL on
port 80. Always re-web-search vendor defaults before a new publishable run.

## What was excluded

An earlier AIPerf exploratory profile used **synthetic** ISL≈550 (not the
prompt-library mix). That run is **not** in the comparison tables. Only the
matched `single_turn` conversion of `mix.jsonl` is used for AIPerf numbers.

## Metrum results (vLLM)

Source: [metrum-summary.json](bakeoff/20260923-pro6000/metrum-summary.json).
All stages: `n=32`, `error_rate=0`.

| Load | tok/s | e2e p50 (ms) | e2e p95 (ms) | prefill p50 (ms) | user tok/s p50 | in-flight mean | cap engagement |
|-----:|------:|-------------:|-------------:|-----------------:|---------------:|---------------:|---------------:|
| 1 | 85.95 | 471.3 | 559.3 | 26.9 | 85.92 | 1.00 | 97% |
| 2 | 169.13 | 441.3 | 557.7 | 33.6 | 85.20 | 1.97 | 94% |
| 4 | 340.26 | 450.6 | 550.1 | 33.1 | 86.56 | 3.83 | 89% |
| 8 | 586.80 | 433.7 | 550.0 | 33.5 | 86.29 | 7.17 | 72% |
| 16 | 1001.17 | 469.3 | 568.5 | 34.8 | 84.41 | 12.56 | 44% |

Metrum knee heuristic: **load 8** at **586.80 tok/s** (not a saturation claim).

## AIPerf results (same prompts, same vLLM)

Source: `aiperf/concurrency_*/profile_export_aiperf.csv`.

| Conc | tok/s | req/s | e2e p50 (ms) | TTFT p50 (ms) | user tok/s p50 | ISL p50 | OSL p50 |
|-----:|------:|------:|-------------:|--------------:|---------------:|--------:|--------:|
| 1 | 85.03 | 2.24 | 440.4 | 22.1 | 88.37 | 256.5 | 38.0 |
| 2 | 167.22 | 4.34 | 427.4 | 32.0 | 89.79 | 256.5 | 36.5 |
| 4 | 325.46 | 8.74 | 415.8 | 31.7 | 91.10 | 256.5 | 36.0 |
| 8 | 626.04 | 16.09 | 430.9 | 32.2 | 91.24 | 256.5 | 37.5 |
| 16 | 1133.25 | 29.08 | 470.7 | 35.3 | 89.11 | 256.5 | 39.5 |

AIPerf OSL p50 landed ~36–40 despite `--output-tokens-mean 48` (early EOS).
Metrum enforced `max_tokens=48` on the same mix. Prefill/TTFT definitions are
not identical across tools; treat as related, not interchangeable.

## Phase timings (wall clock)

From [timings.jsonl](bakeoff/20260923-pro6000/timings.jsonl) plus matched
AIPerf follow-up:

| Phase | Tool | Duration (s) |
|-------|------|-------------:|
| host_prep | shared | 0 |
| install_metrum | metrum | 0 (already on PATH) |
| install_aiperf | aiperf | 25 |
| prompt_extract | metrum | 2 |
| sut_write | shared | 0 |
| metrum_sweep | metrum | 33 |
| aiperf_sweep (synthetic exploratory) | aiperf | 10 |
| aiperf_matched_sweep (prompt-library) | aiperf | 64 |
| client_ceiling (mock) | metrum | 1 |

AIPerf cold `pip` install dominates first-run setup. Matched multi-concurrency
sweep wall time exceeds Metrum's single strategic invocation because AIPerf
runs each concurrency as a separate profile variation with tokenizer/setup
overhead.

## Session capture (UX)

Codex CLI `0.156.1` on the GPU host via Kimi (`kimi-k2.7-code`) through
`@codeproxy/cli` on `:8787`. Recorded with `asciinema` + `script`:

- [codex-metrum.cast](bakeoff/20260923-pro6000/codex/codex-metrum.cast)
- [codex-aiperf.cast](bakeoff/20260923-pro6000/codex/codex-aiperf.cast)
- [bakeoff-session.typescript](bakeoff/20260923-pro6000/bakeoff-session.typescript)

Operator prompts: `scripts/live/bakeoff/CODEX_*.md`.

## UX comparison

| Topic | Metrum | AIPerf |
|-------|--------|--------|
| Install | Static binary / cargo | Python venv + pip (~25 s cold) |
| Prompt library | Native extract + profiles + mix report | Convert JSONL to `single_turn` |
| Publishable stamp | `--sut` / `--require-sut` | Rich artifact dirs; no SUT stamp |
| Sweep UX | One CLI call, knee + HTML/CSV | Per-concurrency artifacts + sweep aggregate |
| Diagnostics | Observed concurrency, connect/prefill/decode, SLOs | Broad HTTP / optional GPU telemetry |

## Client ceiling

Mock-server high-concurrency probe (~1 s) bounds **client** capacity, not the
GPU SUT. Not used as a GPU score.

## Go / no-go

**Go** for Metrum AI Bench CLI for publishable on-host LLM load studies when
you need SUT stamps, prompt-library mixes, strategic knee/HTML, and observed
concurrency.

**Keep AIPerf** for NVIDIA-oriented telemetry breadth and ecosystem familiarity.

Neither tool "wins" absolute tok/s on this short `chat-short` ladder;
agreement under matched ISL/OSL is the useful finding. Prefer loopback /
on-host loadgen for TTFT and concurrency claims.

## Reproduce

On the GPU host:

```bash
export BENCH_URL=http://127.0.0.1/v1/chat/completions
export BENCH_MODEL=Qwen/Qwen2.5-7B-Instruct
./scripts/live/bakeoff/run_on_host.sh
```

Matched AIPerf flags:

```bash
aiperf profile --model Qwen/Qwen2.5-7B-Instruct \
  --url http://127.0.0.1 --endpoint-type chat --streaming \
  --concurrency 1,2,4,8,16 --request-count 32 --warmup-request-count 4 \
  --output-tokens-mean 48 \
  --input-file aiperf-prompts.jsonl --custom-dataset-type single_turn \
  --artifact-dir aiperf-artifacts
```

Closes #144 (epic #148).
