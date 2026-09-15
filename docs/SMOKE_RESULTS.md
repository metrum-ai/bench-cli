<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Smoke results — campaign `v1rc1-20260915-190838`

Consolidated multi-modality smoke after local gates. Raw JSONL stays under
gitignored `live-results/`; this document is the public, releasable summary
(SUT provenance + aggregates). Ship via **GitHub Releases**.

| Field | Value |
|-------|-------|
| Campaign ID | `v1rc1-20260915-190838` |
| Bench package | `metrum-ai-bench-*` **1.0.0-rc.1** |
| Date (UTC) | 2026-09-15 |
| Validation | 10 result files, 474 measured request lines |
| Modalities | llm, vlm |

## Systems under test

### GPU lanes (LLM / VLM)

| Item | Value |
|------|-------|
| Cloud / region | massedcompute / beltsville-usa-1 |
| Instance type | RTXPro6000 / RTXPro6000 |
| GPU | RTXPro6000 ×1 |
| VRAM | not captured GiB |
| Host OS | Shadeform host (not probed) |
| NVIDIA driver | **not captured (no SSH probe before teardown)** |
| Host CUDA (driver) | **not captured** |
| Model server | **vLLM vllm/vllm-openai:latest** |
| Container image | `vllm/vllm-openai:latest` |
| Image digest | `not recorded (Shadeform pull of :latest; digest unavailable after teardown)` |
| Torch / CUDA (container) | — / — |
| LLM model | `Qwen/Qwen2.5-7B-Instruct` |
| VLM model | `Qwen/Qwen2.5-VL-7B-Instruct` |

### ASR / imagegen

Labeled **dummy-certified** when no practical GPU image was available;
validates CLI wiring and schemas only.

## Results

### LLM — closed-loop concurrency

| Cell | n | latency p50 | latency p95 | TTFT p50 | TTFT p95 |
| --- | --- | --- | --- | --- | --- |
| c1-n64 | 56 | 0.248 | 0.251 | 0.056 | 0.060 |
| c2-n64 | 56 | 0.256 | 0.257 | 0.067 | 0.068 |
| c4-n64 | 56 | 0.252 | 0.259 | 0.066 | 0.069 |
| c8-n64 | 56 | 0.251 | 0.258 | 0.065 | 0.076 |

### LLM — open-loop request rate

| Cell | n | latency p50 | latency p95 | TTFT p50 | TTFT p95 |
| --- | --- | --- | --- | --- | --- |
| rate16-n64 | 56 | 0.254 | 0.259 | 0.069 | 0.074 |
| rate4-n64 | 56 | 0.246 | 0.251 | 0.054 | 0.061 |
| rate8-n64 | 56 | 0.259 | 0.264 | 0.069 | 0.075 |

### VLM — concurrency

| Cell | n | latency p50 | latency p95 | TTFT p50 | TTFT p95 |
| --- | --- | --- | --- | --- | --- |
| c1-n32 | 24 | 0.296 | 0.501 | 0.059 | 0.061 |
| c2-n32 | 24 | 0.310 | 0.437 | 0.076 | 0.079 |
| c4-n32 | 24 | 0.289 | 0.370 | 0.076 | 0.079 |

### ASR — dummy-certified

| Cell | n | latency p50 (s) | RTFx client p50 | WER p50 |
| --- | --- | --- | --- | --- |
| — | — | — | — | — |

### Imagegen — dummy-certified

| Cell | Successful images | Images/s | Latency p50 (ms) |
| --- | --- | --- | --- |
| — | — | — | — |

## Reproducing

```bash
./scripts/live/campaign.sh demo
./scripts/live/campaign.sh validate
./scripts/live/campaign.sh report
```

## Throughput recompute (summary.v3 vs started_at+latency_s)

Independent audit: `window_seconds` and `requests_per_second` must match
`max(started_at + latency_s) - min(started_at)` over measured successes within 5%.

| Cell | n | window_s | rps | recompute match |
| --- | --- | --- | --- | --- |
| `llm/c1-n64` | 56 | 13.9019 | 4.028 | yes |
| `llm/c2-n64` | 56 | 7.2206 | 7.756 | yes |
| `llm/c4-n64` | 56 | 3.6003 | 15.554 | yes |
| `llm/c8-n64` | 56 | 1.8236 | 30.708 | yes |
| `llm/rate16-n64` | 56 | 3.7000 | 15.135 | yes |
| `llm/rate4-n64` | 56 | 13.9999 | 4.000 | yes |
| `llm/rate8-n64` | 56 | 7.1298 | 7.854 | yes |
| `vlm/c1-n32` | 24 | 7.5137 | 3.194 | yes |
| `vlm/c2-n32` | 24 | 4.2284 | 5.676 | yes |
| `vlm/c4-n32` | 24 | 1.9744 | 12.155 | yes |

## Provenance notes

- GPU ladder selected **RTXPro6000** on massedcompute / beltsville-usa-1 (no step-down).
- Models: Qwen2.5-7B instruct family (<10B); last-resort documented default for rc smoke.
- Launch flags in `sut.json` / create payloads; image digest not captured for `:latest`.
- Hard GPU lifetime: 4 hours (`ttl.json`); instances torn down after validate+report.
