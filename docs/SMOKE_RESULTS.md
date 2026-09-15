<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Smoke results — campaign `oss-20260915-hostnet`

Consolidated multi-modality smoke after local gates. Raw JSONL stays under
gitignored `live-results/`; this document is the public, releasable summary
(SUT provenance + aggregates). Ship via **GitHub Releases**.

| Field | Value |
|-------|-------|
| Campaign ID | `oss-20260915-hostnet` |
| Bench package | `metrum-ai-bench-*` **0.1.82** |
| Date (UTC) | 2026-09-15 |
| Validation | 15 result files, 583 measured request lines |
| Modalities | llm, vlm, asr, imagegen |

## Systems under test

### GPU lanes (LLM / VLM)

| Item | Value |
|------|-------|
| Cloud / region | Shadeform → Massed Compute / kansascity-usa-6 |
| Instance type | RTXPro6000 / gpu_1x_pro_6000_blackwell |
| GPU | NVIDIA RTX PRO 6000 Blackwell Server Edition ×1 |
| VRAM | 96 GiB |
| Host OS | Ubuntu 22.04.5 LTS (ubuntu22.04_cuda13.0_shade_os) |
| NVIDIA driver | **580.126.09** |
| Host CUDA (driver) | **13.0** |
| Model server | **vLLM 0.29.0** |
| Container image | `vllm/vllm-openai:latest` |
| Image digest | `sha256:c2914767605584b6d8f45686b82de173ecc99e781897aa3d0a66dacd72c51ae1` |
| Torch / CUDA (container) | 2.13.0+cu130 / 13.0 |
| LLM model | `Qwen/Qwen2.5-7B-Instruct` |
| VLM model | `Qwen/Qwen2.5-VL-7B-Instruct` |

### ASR / imagegen

Labeled **dummy-certified** when no practical GPU image was available;
validates CLI wiring and schemas only.

## Results

### LLM — closed-loop concurrency

| Cell | n | latency p50 | latency p95 | TTFT p50 | TTFT p95 |
| --- | --- | --- | --- | --- | --- |
| c1-n64 | 56 | 0.244 | 0.245 | 0.051 | 0.052 |
| c2-n64 | 56 | 0.257 | 0.258 | 0.067 | 0.067 |
| c4-n64 | 56 | 0.253 | 0.254 | 0.066 | 0.067 |
| c8-n64 | 56 | 0.251 | 0.258 | 0.066 | 0.072 |

### LLM — open-loop request rate

| Cell | n | latency p50 | latency p95 | TTFT p50 | TTFT p95 |
| --- | --- | --- | --- | --- | --- |
| rate16-n64 | 56 | 0.253 | 0.258 | 0.067 | 0.071 |
| rate4-n64 | 56 | 0.243 | 0.244 | 0.051 | 0.053 |
| rate8-n64 | 56 | 0.258 | 0.263 | 0.068 | 0.073 |

### VLM — concurrency

| Cell | n | latency p50 | latency p95 | TTFT p50 | TTFT p95 |
| --- | --- | --- | --- | --- | --- |
| c1-n32 | 24 | 0.258 | 0.424 | 0.053 | 0.054 |
| c2-n32 | 24 | 0.292 | 0.358 | 0.067 | 0.068 |
| c4-n32 | 24 | 0.276 | 0.350 | 0.066 | 0.067 |

### ASR — dummy-certified

| Cell | n | latency p50 (s) | RTFx client p50 | WER p50 |
| --- | --- | --- | --- | --- |
| c1-n32 | 24 | 0.101 | 39.73 | 0.0 |
| c2-n32 | 24 | 0.101 | 39.72 | 0.0 |
| c4-n32 | 24 | 0.101 | 39.69 | 0.0 |

### Imagegen — dummy-certified

| Cell | Successful images | Images/s | Latency p50 (ms) |
| --- | --- | --- | --- |
| c1-n16 | 12 | 7.370 | 101.419 |
| c2-n16 | 12 | 14.745 | 101.415 |

## Reproducing

```bash
./scripts/live/campaign.sh demo
./scripts/live/campaign.sh validate
./scripts/live/campaign.sh report
```

