<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Smoke results - campaign `matrix-20260915-195537`

> **Coverage:** the published smoke matrix is currently **NVIDIA-only** (L40S, RTX PRO 6000 via Shadeform / Massed Compute). AMD Instinct coverage is in progress and will be added under the same manifest standard. No comparative vendor results are published here.

Shadeform smoke against the **Test Matrix for Metrum Bench CLI** (PERFORMANCE TESTS).
Raw JSONL under gitignored `live-results/`; this document is the public summary.

| Field | Value |
|-------|-------|
| Campaign ID | `matrix-20260915-195537` |
| Bench package | `metrum-ai-bench-*` **1.0.0-rc.1** (campaign `matrix-20260915`; see also RTX PRO 6000 campaign `v1rc1-20260915-190838` in CHANGELOG) |
| Date (UTC) | 2026-09-15 |
| Engine | `vllm` / `vllm/vllm-openai:latest` |
| Validation | 15 result files |
| Modalities | asr, llm, vlm |

## Systems under test

| Item | Value |
|------|-------|
| Cloud / region | massedcompute (desmoines / kansascity) |
| LLM/VLM SKU | L40Sx2 (TP=2) |
| ASR SKU | L40S |
| LLM models | `google/gemma-4-12B-it`, `Qwen/Qwen3.8-27B-FP8` |
| VLM models | `google/gemma-4-12B-it`, `Qwen/Qwen3.8-27B-FP8` |
| ASR model | `openai/whisper-large-v3` |
| Imagegen | deferred (`stabilityai/stable-diffusion-3.5-large`) |
| Target ISL×OSL | 1024 × 1024 |
| Sheet concurrencies | LLM 32/64/128; VLM 8/16/32; ASR 32/64/128 |

### Launch flags

> Publication runs should pass `--sut sut.json --require-sut` (rc.5+). Hostname redaction is implied.

- **llm/vlm (gemma then Qwen recreate)**: `--model <id> --host 0.0.0.0 --port 8000 --tensor-parallel-size 2` on `L40Sx2`
- **asr**: `--model openai/whisper-large-v3 --host 0.0.0.0 --port 8000` on `L40S`

## Results

TTFT columns are `ttft_s` (first visible token) from tool `summary.v3` / request `ttft_s` (never `first_byte_s`). When JSONL is absent, TTFT is recovered from cell `stdout.txt`.

### LLM - closed-loop (sheet conc 32/64/128)

| Model | Cell | n | err | lat p50 | lat p95 | TTFT p50 | TTFT p95 | window_s | rps | recompute |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `Qwen_Qwen3.8-27B-FP8` | c128 | 254 | 2 | 93.124 | 169.348 | 1.474 | 33.125 | 230.2267 | 1.103 | yes |
| `Qwen_Qwen3.8-27B-FP8` | c32 | 64 | 0 | 36.009 | 53.124 | 0.622 | 7.308 | 97.2582 | 0.658 | yes |
| `Qwen_Qwen3.8-27B-FP8` | c64 | 128 | 0 | 50.181 | 85.189 | 1.141 | 15.719 | 131.4558 | 0.974 | yes |
| `google_gemma-4-12B-it` | c128 | 256 | 0 | 21.788 | 42.760 | 1.146 | 15.567 | 52.3483 | 4.890 | yes |
| `google_gemma-4-12B-it` | c32 | 64 | 0 | 7.355 | 14.154 | 0.449 | 5.264 | 21.1478 | 3.026 | yes |
| `google_gemma-4-12B-it` | c64 | 128 | 0 | 11.992 | 22.632 | 0.616 | 7.526 | 30.4229 | 4.207 | yes |

### VLM - closed-loop (sheet conc 8/16/32)

| Model | Cell | n | err | lat p50 | lat p95 | TTFT p50 | TTFT p95 | window_s | rps | recompute |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `Qwen_Qwen3.8-27B-FP8` | c16 | 32 | 0 | 6.960 | 8.662 | 0.331 | 1.219 | 20.4994 | 1.561 | yes |
| `Qwen_Qwen3.8-27B-FP8` | c32 | 64 | 0 | 9.814 | 13.639 | 0.647 | 2.565 | 25.3621 | 2.523 | yes |
| `Qwen_Qwen3.8-27B-FP8` | c8 | 16 | 0 | 6.046 | 8.473 | 0.321 | 0.539 | 14.7532 | 1.085 | yes |
| `google_gemma-4-12B-it` | c16 | 32 | 0 | 1.627 | 2.029 | 0.271 | 0.794 | 4.8716 | 6.569 | yes |
| `google_gemma-4-12B-it` | c32 | 64 | 0 | 2.072 | 2.162 | 0.369 | 0.601 | 5.3183 | 12.034 | yes |
| `google_gemma-4-12B-it` | c8 | 16 | 0 | 1.391 | 1.452 | 0.241 | 0.282 | 2.9095 | 5.499 | yes |

### ASR - closed-loop (sheet conc 32/64/128)

| Model | Cell | n | err | lat p50 | lat p95 | TTFT p50 | TTFT p95 | window_s | rps | recompute |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `openai_whisper-large-v3` | c128 | 0 | 264 | - | - | - | - | 0.7009 | 0.000 | - |
| `openai_whisper-large-v3` | c32 | 0 | 72 | - | - | - | - | 0.6473 | 0.000 | - |
| `openai_whisper-large-v3` | c64 | 0 | 136 | - | - | - | - | 1.1839 | 0.000 | - |

Whisper `/v1/models` answered, but every transcription upload returned **HTTP 400** `Invalid or unsupported audio file` (ffmpeg wav/mp3/flac and repo `dummy.mp3`). Probe evidence: `asr/probe/`. Cells retained as all-error measurements.

### Image generation

Not executed on Shadeform (no OpenAI `/v1/images/generations` docker path in create helper). Status: `imagegen/planned/status.json`.

## Coverage vs sheet

| Row | Sheet | This campaign |
| --- | --- | --- |
| 1 Text / LLM | gemma-4-12B-it + Qwen3.8-27B-FP8; vLLM+SGLang; 2×L40S; 1024×1024; c=32,64,128 | **vLLM** gemma + Qwen on 2×L40S; ISL pad≈1024 / OSL=1024; all sheet concs. **SGLang not run** (honest gap). |
| 2 VLM | same models/frameworks; c=8,16,32 | **vLLM** gemma + Qwen on 2×L40S; all sheet concs. SGLang not run. |
| 3 ASR | whisper-large-v3; 1×L40S; c=32,64,128 | **vLLM** whisper on 1×L40S; cells executed; **0 successful transcriptions** (server rejects audio). |
| 4 Imagegen | SD3.5-large; 1×L40S; 8K; c=2,4,8 | **Deferred**. |

## Throughput recompute

Independent window/rps recomputed from `request.v3` measure rows vs `summary.v3` (5% relative tolerance). See `recompute` column above; aggregate at `live-results/campaign-matrix-20260915-195537/aggregate.json`.

## Provenance notes

- Hard TTL 6h in `ttl.json`.
- Qwen pass: gemma LLM/VLM instances deleted; new L40Sx2 pair loaded `Qwen/Qwen3.8-27B-FP8`; ASR instance retained through Qwen sweep.
- Script: `scripts/live/matrix_smoke.sh` (+ `shadeform.sh` model/extra-args).
- Secrets: `env.json` never printed or committed.
- N-01: never publish `first_byte_s` as TTFT; TTFT is always `ttft_s`.
