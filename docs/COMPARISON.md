<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Comparison with other inference measurement tools

## 1. What this document is

This is a scoped comparison of **client-side** inference measurement tools,
compiled **2026-09** from public project pages and documentation. It is not a
hands-on bake-off: no tool was re-run here for this rewrite. Metric names only
match when tokenizer, prompt sequence, sampling parameters, warmup, measurement
window, endpoint topology, and SLO definitions match.

Comparative behavior remains untested until a controlled side-by-side run
exists.

## 2. What Metrum AI Bench CLI ships

One client process (`metrum-ai-bench-cli` with modality subcommands; see
[CLI.md](CLI.md) and [STRATEGIC_BENCHMARKING.md](STRATEGIC_BENCHMARKING.md)):

| Capability | Where |
|------------|--------|
| Four OpenAI-compatible modalities: LLM, VLM, ASR, imagegen | `metrum-ai-bench-cli {llm,vlm,asr,imagegen}` |
| Open-loop scheduling with seeded Poisson (or constant) arrivals | `--request-rate`, `--arrival poisson\|constant`, `--seed` |
| Pooled multi-endpoint distribution | `--endpoints-file` / multi-URL; aggregates labeled `pooled_mixture` ([METRICS.md](METRICS.md)) |
| TTFT = first **visible** output token (not first-byte headers) | `ttft_s`; headers are `first_byte_s` ([METRICS.md](METRICS.md)) |
| Warmup excluded from measured distributions | `--warmup-requests` |
| Goodput requires stated SLOs | `--slo ttft=\|tpot=\|e2e=`; without SLOs goodput equals validity-filtered throughput on strategic |
| Partial-run flag | Ctrl-C / SIGTERM drain writes `partial: true` on the summary |
| p99 flagged unreliable at low *n* | `p99_unreliable` when *n* &lt; 100 ([METRICS.md](METRICS.md), `src/stats.rs`) |
| Hyndman–Fan type 7 percentiles | `percentile_method: hyndman_fan_type7` |
| Concurrency/rate sweeps, HTML report, MLPerf-shaped export | strategic: `--sweep`, `--html`, `--mlperf-dir` |

## 3. Landscape

| Tool | What it is | What it answers | Overlap with Metrum AI Bench CLI | What Metrum AI Bench CLI adds | What it does better than Metrum AI Bench CLI |
|------|------------|-----------------|----------------------------------|-------------------------------|---------------------------------------------|
| **SemiAnalysis InferenceX** (formerly InferenceMAX) | Public continuous inference benchmarking program and reporting surface | How named stacks score on a published, recurring hardware CI matrix | Client-visible latency/throughput language; interest in real serving | Local, operator-owned runs with a stamped manifest you control; four OpenAI-compatible modalities in one client | Continuous public CI on real hardware; a recognized public scoreboard beats a self-reported local manifest for industry narrative |
| **NVIDIA AIPerf** (successor to GenAI-Perf (retired; see AIPerf)) | NVIDIA's generative-AI performance client and reporting stack | How a Triton / NIM / NVIDIA-oriented serving path behaves under load with rich telemetry | Request rate, Poisson-style arrivals, tokenizer counts, SLO-style goodput ideas; generative modalities including LLM, VLM, ASR, image, and video | Pooled multi-endpoint mixture labeling; strategic `--sweep` / `--html` / `--mlperf-dir`; operator-owned stamped `summary.v3` / SUT. Modality breadth is not a differentiator: AIPerf already ships ASR, image, video, and VLM | Broader ecosystem familiarity and telemetry; video and deeper NVIDIA CI / partner wiring Metrum AI Bench CLI does not match |
| **MLPerf Inference** | Audited industry suite (LoadGen + compliance) | Whether a submission meets a fixed, audited workload and accuracy rules | Latency/throughput language; Server/Offline scenario names | Informal LoadGen-shaped **export** for parser experiments (`--mlperf-dir`); not a substitute for LoadGen | Audited rules, accuracy requirements, and submission standing Metrum AI Bench CLI does not have |
| **vLLM / SGLang `bench_serving`** | Engine-adjacent serving microbenchmarks | How *this* engine build behaves on *these* datasets at a chosen rate/concurrency | `--request-rate`, concurrency caps, seeds, ignore-EOS, local tokenizer counts | Cross-engine OpenAI-compatible client; multi-modality; strategic sweeps and exports | Already installed next to the engine; deep dataset integrations for that stack |
| **[GuideLLM 0.7](https://github.com/vllm-project/guidellm/releases/tag/v0.7.0) (`vllm-project`) / LLMPerf** | Python load / perf clients aimed at LLM HTTP APIs | Latency and throughput under scripted load, often with sweep/report UX | Rate/concurrency sweeps; HTML-style reporting (GuideLLM) | Shared Rust client across four modalities; typed errors; stamped `summary.v3` config; open-loop coordinated-omission fields | GuideLLM's sweep/report packaging and LLMPerf's Python ecosystem fit for quick scripts |
| **lm-evaluation-harness / HELM / OpenCompass** | Accuracy and capability evaluation harnesses | Quality and task scores, not serving goodput under SLOs | Prompt corpora and model IDs sometimes shared informally | Metrum AI Bench CLI measures serving performance (plus ASR WER/CER only); these measure task quality | Breadth of quality tasks, leaderboards, and academic comparison protocols |
| **NVIDIA Dynamo** | Inference serving / runtime stack | How to *run* multi-node inference efficiently | You may point Metrum AI Bench CLI **at** Dynamo-fronted OpenAI-compatible endpoints | Dynamo is a **measurement target**, not an alternative measurement tool | End-to-end serving product features Metrum AI Bench CLI does not replace |

## 4. MLPerf export

`metrum-ai-bench-cli-strategic --mlperf-dir …` writes LoadGen-shaped text for
parser-oriented interoperability. It is **not** an audited submission and is
**not** MLPerf-compatible in the compliance sense.

From `src/strategic.rs`:

```text
UNOFFICIAL: This is NOT an audited or submitted MLPerf result. Parser-oriented interoperability export only; do not treat as an official MLPerf LoadGen run.
```

Validity lines use `UNOFFICIAL_OK` / `UNOFFICIAL_INVALID` and **refuse** the
official LoadGen substring `Result is : VALID` (covered in
`tests/e2e_strategic.rs`). See also
[STRATEGIC_BENCHMARKING.md](STRATEGIC_BENCHMARKING.md).

## 5. What we do not claim

- Not first, only, or industry-standard.
- A stamped manifest is table stakes for publishable runs, not a moat.
- Not a competitor to MLPerf Inference submissions.
- Not a head-to-head performance bake-off against AIPerf or InferenceX.
- No claim that Metrum AI Bench CLI replaces engine-native `bench_serving` for
  engine developers who already live in that tree.

## 6. Not in 1.0 (roadmap)

Planned for a **next release** (no dates):

- Agent mode
- Quality metrics beyond ASR WER/CER
- Cost per accepted task
