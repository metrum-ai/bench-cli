<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Smoke results - campaign `matrix-20260915-195537`

> **Coverage:** the published smoke matrix is currently **NVIDIA-only** (L40S, RTX PRO 6000 via Shadeform / Massed Compute). AMD Instinct coverage is in progress and will be added under the same manifest standard. No comparative vendor results are published here.

Shadeform smoke against the **Test Matrix for Metrum AI Bench CLI**
(PERFORMANCE TESTS). Measured with package **1.0.0-rc.1** (before
`--require-sut`). This page is a smoke summary, not a publication under
[RESULTS_PUBLICATION_POLICY.md](RESULTS_PUBLICATION_POLICY.md). Raw JSONL is
under gitignored `live-results/`. To re-run on a current release with
`--sut --require-sut`, see `make smoke-regen` / `scripts/live/regen_smoke.sh`.

| Field | Value |
|-------|-------|
| Campaign ID | `matrix-20260915-195537` |
| Bench package | `metrum-ai-bench-cli-*` **1.0.0-rc.1** (campaign `matrix-20260915`; see also RTX PRO 6000 campaign `v1rc1-20260915-190838` in CHANGELOG) |
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

> `--sut` / `--require-sut` arrived in rc.5+. Use them for publication runs.

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

---

# Smoke results - campaign `widen-shadeform-20261002`

> **Coverage:** Shadeform parallel widen across LLM, VLM, ASR, and ImageGen on
> 1x H100 PCIe (massedcompute / Scaleway). Not a publication under
> [RESULTS_PUBLICATION_POLICY.md](RESULTS_PUBLICATION_POLICY.md). Raw logs live
> under gitignored `artifacts/live/`. Combined operator report:
> `/tmp/bench-cli-shadeform-smoke-report.md` (plus per-lane
> `/tmp/bench-cli-{vlm,asr,imagegen}-smoke-report.md`).

| Field | Value |
|-------|-------|
| Campaign ID | `widen-shadeform-20261002` |
| Bench package | `metrum-ai-bench-cli-*` from `fix/modality-validation` @ `61fa69e` (version string `1.5.2`; not the 1.5.2 release tag) |
| Date (UTC) | 2026-10-02 |
| Engines | LLM/VLM/ASR: `vllm/vllm-openai:v0.30.0`; ImageGen: `vllm/vllm-omni:v0.30.0` |
| Telemetry | Metrum all-smi fork `v0.26.3-metrum.4` at `http://127.0.0.1:9090/metrics` |
| Modalities | llm, vlm, asr, imagegen |

## Systems under test

| Modality | Cloud / SKU | Model | Launch notes |
|----------|-------------|-------|--------------|
| LLM | Scaleway `paris-france-1`, 1x H100 PCIe ($3.30/h) | `Qwen/Qwen3.8-27B-FP8` | `--reasoning-parser qwen3 --language-model-only --max-model-len 131072 --gpu-memory-utilization 0.90 --max-num-seqs 256`; thinking off via `chat_template_kwargs.enable_thinking=false` |
| VLM | massedcompute `desmoines-usa-1`, 1x H100 PCIe ($2.73/h) | `Qwen/Qwen3-VL-32B-Instruct-FP8` | `--max-model-len 32768 --limit-mm-per-prompt.video 0 --async-scheduling --mm-processor-cache-gb 0` |
| ASR | massedcompute `desmoines-usa-1`, 1x H100 PCIe ($2.73/h) | `openai/whisper-large-v3-turbo` | plain vLLM STT, `--max-model-len 448` (Omni ASR blocked by vllm-omni#5722) |
| ImageGen | massedcompute `desmoines-usa-1`, 1x H100 PCIe ($2.73/h) | `Tongyi-MAI/Z-Image-Turbo` | `vllm serve ... --omni --port 8000` |

All measured cells used `--sut` / `--require-sut`. Instances deleted after each lane.

## Results

TTFT is always `ttft_s` (first visible streamed token), never `first_byte_s`.

### LLM

Artifacts: `artifacts/live/widen-20261002T144347Z/`.

| Cell | n | err | Output tok/s | TTFT p50 (s) | Notes |
| --- | --- | --- | --- | --- | --- |
| preflight | - | - | - | - | FAIL (streaming_first_token empty; fixed later with preflight `--extra-body-json`) |
| G5 | 200 | 0 | 54.542 | 0.0554 | PASS |
| MB | 200 | 0 | 54.558 | 0.0501 | PASS |
| strategic c=1 | 48 | 0 | 54.579 | 0.0503 | PASS |
| strategic c=4 | 48 | 0 | 194.835 | 0.0794 | PASS |
| strategic c=16 | 48 | 0 | 620.258 | 0.2069 | PASS |

Prompts from `metrum-ai/prompt-library` (`sample`, ISL 256 / OSL 128).

### VLM

Artifacts: `artifacts/live/widen-parallel-vlm-20261002T153627Z/`.

| Cell | n | err | Completion tok/s | Req/s | TTFT p50 / p95 (s) | E2E p50 / p95 (s) |
| --- | --- | --- | --- | --- | --- | --- |
| vlm-c8-a1 (gate) | 64 | 0 | 329.043 | 3.113 | 0.141 / 0.237 | 2.518 / 2.704 |
| vlm-c16-a1 | 64 | 0 | 579.148 | 5.468 | 0.213 / 0.398 | 2.807 / 3.086 |
| vlm-c32-a1 | 64 | 0 | 867.291 | 8.043 | 0.391 / 0.649 | 3.479 / 4.208 |

### ASR

Artifacts: `artifacts/live/widen-parallel-asr-20261002T153555Z/`. Fixtures: `test-data/asr/`.

| Cell | n | err | Req/s | Latency p50 (s) | Mean WER / CER |
| --- | --- | --- | --- | --- | --- |
| asr-c1-a1 (gate) | 60 | 0 | 12.953 | 0.0668 | 0.0000 / 0.0000 |
| asr-c8-a1 | 960 | 0 | 42.663 | 0.1554 | 0.0000 / 0.0000 |

Independent verbose_json probe also scored WER 0.0000.

### Image generation

Artifacts: `artifacts/live/widen-parallel-imagegen-20261002T153737Z/`. Smoke-scale only (not a performance claim).

| Cell | n | err | Latency p50 (s) | Req/s | Decoded artifacts |
| --- | --- | --- | --- | --- | --- |
| c2 (gate) | 4 | 0 | 3.013 | 0.525 | 5/5 |
| c1 | 8 | 0 | 1.547 | 0.640 | 9/9 |

As-run note: `--prompt` + `--seed` under default `--seed-mode increment` sent a constant seed (findings PR fixes client increment).

## Provenance notes

- Operator LLM lane plus parallel Herdr agents `smoke-vlm`, `smoke-asr`, `smoke-img`.
- Secrets: `env.json` never printed or committed.
- N-01: never publish `first_byte_s` as TTFT; TTFT is always `ttft_s`.
- Findings follow-ups (same date): all-smi scrape path `/metrics`, `analyze.py` power allowlist + millijoule scale, Omni metric filter in `widen_cell.sh`, ASR torchcodec triage, shadeform key preference, preflight `--extra-body-json`, imagegen seed increment.

---

# Smoke results - campaign `publish-20261002T162512Z`

> Four-modality Shadeform publish widen run from the operator host after the
> modality-gap stack landed on `main` (`ca51dd5`). Full narrative:
> `artifacts/live/publish-20261002T162512Z/REPORT.md`.

| Field | Value |
|-------|-------|
| Campaign ID | `publish-20261002T162512Z` |
| Bench package | tip `v1.5.2-9-gca51dd5` (`ca51dd5`) |
| Date (UTC) | 2026-10-02 |
| Engines | LLM/VLM/ASR: `vllm/vllm-openai:v0.30.0`; ImageGen: `vllm/vllm-omni:v0.30.0` |
| Telemetry | all-smi `v0.26.3-metrum.4` `/metrics` |
| Modalities | llm, vlm, asr, imagegen |

## Headline

| Modality | SKU | Model | Gate cell | Result |
|---|---|---|---|---|
| LLM | 1x H100 PCIe | `Qwen/Qwen3.8-27B-FP8` | G5 / MB | 200/200 each; **52.357 / 52.474** output tok/s; TTFT p50 ~0.073 s |
| VLM | 1x H100 PCIe | `Qwen/Qwen3-VL-32B-Instruct-FP8` | c8 / c16 / c32 | 64/128/256 ok; **360.5 / 697.8 / 1278.9** completion tok/s |
| ASR | 1x H100 PCIe | `openai/whisper-large-v3-turbo` | c1 / c8 | 60/960 ok; **WER/CER 0.0000**; 13.6 / 42.7 req/s |
| ImageGen | 1x L40S | `Tongyi-MAI/Z-Image-Turbo` | c2 / c1 | 10/8 ok; unique increment seeds; 0.315 / 0.331 req/s |

LLM strategic sweep not claimed (remote `--extra-body-json` quoting failure after G5/MB). All instances deleted.

---

# Smoke results - campaign `epic184-h100-20261005`

> Live validation of epic #184 (issues #189 to #199) on Shadeform 1x H100 PCIe,
> all four modalities, with in-binary `--telemetry` / `--ndjson` and strategic
> sweeps for every modality. A validation run, not a publication under
> [RESULTS_PUBLICATION_POLICY.md](RESULTS_PUBLICATION_POLICY.md). Committed,
> redacted bundle with per-issue results, check outputs and exact commands:
> [`artifacts/live/epic184-h100-20261005/REPORT.md`](../artifacts/live/epic184-h100-20261005/REPORT.md).

| Field | Value |
|-------|-------|
| Campaign ID | `epic184-h100-20261005` |
| Bench package | built from `main` at `6f150f3` (tip build; `--version` prints `1.5.3`) |
| Date (UTC) | 2026-10-05 (instance 21:10Z to 21:35Z) |
| System | Shadeform, Scaleway `paris-france-1`, 1x H100 PCIe, driver 580.126.20, Ubuntu 24.04 |
| Engines | LLM/VLM/ASR: `vllm/vllm-openai:v0.31.0`; ImageGen: `vllm/vllm-omni:v0.30.0` |
| Telemetry | all-smi `v0.26.3-metrum.4` `/metrics` plus engine `/metrics`, in-binary |
| Modalities | llm, vlm, asr, imagegen |
| Not run | RTX PRO 6000 Blackwell Server Edition and H200: no Shadeform availability at 2026-10-05 21:08Z or 2026-10-06 00:02Z |

## Headline

| Modality | Model | Cell | Result | Proof (in the bundle) |
|---|---|---|---|---|
| LLM | `Qwen/Qwen3-8B` | R1 c=16, thinking off | 480/480 ok; 1450.471 completion tok/s; TTFT p50 0.031 s | `llm/r1-plain.stdout` |
| LLM | `Qwen/Qwen3-8B` | R2 c=8, thinking on, max 2048 | 51/56 ok (5 `no_output_token` at the cap); 22460 reasoning tokens | `llm/r2-reasoning.stdout` |
| LLM | `Qwen/Qwen3-8B` | sweep c=1..64 | 0 errors; 101.3 to 4948.1 completion tok/s; knee c=32 | `llm/r3-sweep.stdout.json` |
| VLM | `Qwen/Qwen3-VL-8B-Instruct` | c=8 / sweep c=1..16 | 56/56 ok, 750.446 completion tok/s / 0 errors, 0.94 to 12.37 req/s | `vlm/run.stdout`, `vlm/sweep.stdout.json` |
| ASR | `openai/whisper-large-v3-turbo` | c=8 / sweep c=1..16 | 96/96 ok, WER/CER 0.0 / 0 errors, 19.5 to 63.1 req/s | `asr/run.stdout`, `asr/sweep.stdout.json` |
| ImageGen | `Tongyi-MAI/Z-Image-Turbo` | c=1 / sweep c=1..5 | 8/8 ok, 8 distinct PNG sha256 / 0 errors, 6 distinct digests per stage | `imagegen/run.stdout`, `imagegen/images.sha256` |

Findings: #230 (preflight streaming probe on thinking models), #231 (analyze.py histogram
percentiles below the first bucket; fixed by #234), #232 (knee on the nearly linear VLM sweep).
Three operator errors (duplicate telemetry source, base URL instead of endpoint, `--guidance`
instead of `--guidance-scale`) were fixed and rerun on the same instance; see the report.
Instance deleted and verified via `GET /instances/<id>/info`.

---

# Smoke results - campaign `verify233-rtxpro6000-20261006`

> Live re-verification of the #233 fixes (#226, #224, #232, #227, #230, #231, #216, #242) on Shadeform
> 1x RTX PRO 6000 Blackwell Server Edition, all four modalities, with strategic sweeps and `--csv` for
> every modality. A validation run, not a publication under
> [RESULTS_PUBLICATION_POLICY.md](RESULTS_PUBLICATION_POLICY.md). Committed, redacted bundle with
> per-fix results, check outputs and exact commands:
> [`artifacts/live/verify233-rtxpro6000-20261006/REPORT.md`](../artifacts/live/verify233-rtxpro6000-20261006/REPORT.md).

| Field | Value |
|-------|-------|
| Campaign ID | `verify233-rtxpro6000-20261006` |
| Bench package | built from `main` at `d43d9bc` (tip build; `--version` prints `1.5.3`) |
| Date (UTC) | 2026-10-06 (instance 01:54Z to 02:26Z, about $1.17 at $2.19/h) |
| System | Shadeform, massedcompute `beltsville-usa-1`, 1x RTX PRO 6000 Blackwell Server Edition (96 GB, 600 W), driver 580.126.09, Ubuntu 22.04 (dispatcher run in an `ubuntu:24.04` container for glibc 2.39) |
| Engines | LLM/VLM/ASR: `vllm/vllm-openai:v0.31.0`; ImageGen: `vllm/vllm-omni:v0.30.0` |
| Telemetry | all-smi `v0.26.3-metrum.4` `/metrics` plus engine `/metrics`, in-binary |
| Modalities | llm, vlm, asr, imagegen |
| Not run | H200: no Shadeform availability at 2026-10-05 21:08Z, 2026-10-06 00:02Z, 01:43Z or 01:54Z |

## Headline

| Modality | Model | Cell | Result | Proof (in the bundle) |
|---|---|---|---|---|
| LLM | `Qwen/Qwen3-8B` | c=16, 4 warmup, thinking off | 252/252 ok; 21.325 req/s; warmup-to-measured gap 0.154 ms (#226) | `llm/f226.stdout`, `checks.txt` |
| LLM | `Qwen/Qwen3-8B` | sweep c=1..64 | 0 errors; 1.495 to 65.351 req/s; knee c=16 (`kneedle`) | `llm/llm-sweep.stdout.json` |
| VLM | `Qwen/Qwen3-VL-8B-Instruct` | c=8 / 2048x2048 c=4 / sweep c=1..16 | 56/56 ok / 14/14 ok (#242) / 0 errors, no knee (`no_bend`, p95 +10.8%) | `vlm/run.stdout`, `vlm/big.stdout`, `vlm/sweep.stdout.json` |
| ASR | `openai/whisper-large-v3-turbo` | c=8 / sweep c=1..16 | 96/96 ok, WER/CER 0.0 / 0 errors, knee c=2 (saturation) | `asr/run.stdout`, `asr/sweep.stdout.json` |
| ImageGen | `Tongyi-MAI/Z-Image-Turbo` | c=1 / sweep c=1..5 | 8/8 ok, 8 distinct PNG sha256 / 0 errors, flat 0.615 to 0.632 req/s, knee c=1 (saturation) | `imagegen/run.stdout`, `imagegen/sweep.stdout.json` |

`checks.txt`: 11 PASS, 0 FAIL as run. Seven fixes (#226, #224, #232, #227, #230, #231, #216) are verified
with a proving number. #224: the as-run checker's #224 test compared 0 stages (it keyed on a missing CSV
column). The corrected `check_verify_fixed.py` compares 22/22 stages, with a max mismatch of 4.35e-15
(`checks_fixed.txt`, 14 PASS). The imagegen c=5 point that read 0.999 req/s on H100 now reads 0.628. #227:
the live telemetry abort wrote `summary.v3` and a `partial=true` NDJSON summary. A client-side rerun
(`sigint227-rerun/`, mock server, same binaries) showed that one SIGINT during the drain still writes the
summary (exit 1) and two SIGINTs hard-exit 130. #242 works live with a 2.2 MB image body. Because of an
operator copy error, the VLM and ASR SUT `notes` say "H100"; their `gpu` fields are correct.
Instance deleted and verified via `GET /instances/<id>/info`.
