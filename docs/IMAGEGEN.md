<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Image generation benchmark

`metrum-ai-bench-cli-imagegen` measures OpenAI-compatible `POST /v1/images/generations` endpoints. It records latency, images per second, the bytes returned, and a hash for every saved artifact.

**Search first.** Before a real run, web-search the current vLLM-Omni docs
and the model card for the exact diffusion model. Check the supported
models, the image tag, recommended steps, guidance, and resolution, and the
request fields the server accepts. Record them in the SUT. Defaults here are
for `Tongyi-MAI/Z-Image-Turbo` on vllm-omni v0.30.0 only.

**Status:** image generation is supported in code but has **not been verified against a real server**; see [CLAIMS_LEDGER.md](CLAIMS_LEDGER.md). Do not publish image-generation numbers until the live gate (`.github/workflows/live-modality-smoke.yml`, cell `imagegen`) passes and the ledger row is updated.

## What to run

Serve the model with **vLLM-Omni**, then point the bench at the server's `/v1` base URL. Plain `vllm/vllm-openai` does not serve diffusion models.

```bash
scripts/live/serve/imagegen.sh start        # vllm/vllm-omni:v0.30.0, Tongyi-MAI/Z-Image-Turbo, writes a SUT
metrum-ai-bench-cli-imagegen --url http://127.0.0.1:8000/v1 --api-key dummy \
  --model Tongyi-MAI/Z-Image-Turbo --scenario imagegen-smoke \
  --prompt "A red circle, a blue square, and a green triangle on a white background" \
  --size 1024x1024 --num-inference-steps 9 --guidance-scale 0.0 --seed 7 \
  --num-requests 8 --concurrency 1 --warmup-requests 1 \
  --artifact-dir imagegen-artifacts --data-log imagegen.jsonl \
  --sut live-results/serve-imagegen/sut.json --require-sut
cargo xtask assert-headline imagegen imagegen.jsonl --artifact-dir imagegen-artifacts
```

`scripts/live/local_smoke.sh --local --modality imagegen` runs the same cell end to end.

## Serving stack

The launcher [`scripts/live/serve/imagegen.sh`](../scripts/live/serve/imagegen.sh) runs:

```bash
docker run --gpus device=0 --ipc=host -p 8000:8000 vllm/vllm-omni:v0.30.0 \
  vllm serve Tongyi-MAI/Z-Image-Turbo --omni --port 8000
```

- **Image tag:** `vllm/vllm-omni:v0.30.0` is the version we pin. The image has no default entrypoint, so the command must include `vllm serve ... --omni`.
- **Upstream references:**
  - API fields and response shape: [vLLM-Omni image generation API](https://docs.vllm.ai/projects/vllm-omni/en/latest/serving/image_generation_api/)
  - Installation and Docker: [vLLM-Omni docs](https://docs.vllm.ai/projects/vllm-omni/en/latest/) and the CUDA install page at the pinned tag ([`docs/getting_started/installation/gpu/cuda.inc.md` @ v0.30.0](https://github.com/vllm-project/vllm-omni/blob/v0.30.0/docs/getting_started/installation/gpu/cuda.inc.md))
  - Release: [vllm-omni v0.30.0](https://github.com/vllm-project/vllm-omni/releases/tag/v0.30.0)
  - Model: [`Tongyi-MAI/Z-Image-Turbo`](https://huggingface.co/Tongyi-MAI/Z-Image-Turbo). The model card recommends 9 inference steps, guidance 0.0, and 1024x1024.
- **No vendor recipe for 80 GB data-center GPUs.** The vLLM-Omni Z-Image recipe covers offline inference on Intel XPU only. Treat the launcher flags as our choice, and say so in the SUT notes.

Other OpenAI-compatible image servers work too, as long as they implement `/v1/images/generations` and return `b64_json`. Record the server and its version in the SUT.

## Request knobs that change the result

The harness sends what you pass and nothing else. Server defaults often differ from the model card, so set these explicitly on every run:

| Flag | Request field | Why it matters |
|---|---|---|
| `--size WxH` | `size` | Latency scales with pixel count. The harness defaults to `1024x1024`. |
| `--num-inference-steps N` | `num_inference_steps` | **vLLM-Omni's Z-Image pipeline uses 50 steps when the field is absent, while Turbo is designed for 9.** Omitting the flag makes the run roughly 5.5x slower and not comparable. |
| `--guidance-scale G` | `guidance_scale` | Turbo models expect 0.0. |
| `--seed S`, `--seed-mode fixed\|increment\|prompt` | `seed` | Without a seed the server picks a random one. The `increment` default gives each request seed `S + i`. |
| `--n N` | `n` | Images per request (vLLM-Omni accepts 1 to 10); throughput is reported per image. |
| `--negative-prompt`, `--true-cfg-scale` | same | Model-specific. Leave them unset unless the model card uses them. |
| `--response-format b64_json` | `response_format` | vLLM-Omni returns `b64_json` (the default) or raw file bytes, never URLs, so keep `b64_json` against it. |
| `--extra-body-json` / `--extra-body-file` | merged into the body | For other fields such as `flow_shift`. The merged body is recorded in `config.body_template`. |
| `--artifact-dir`, `--no-save-images` | none (harness only) | Decoded images are written here and hashed. `assert_headline.sh --artifact-dir` fails the cell if none decode as PNG or JPEG. |

Prompts can come from `--prompt` (one string) or a `--prompts` JSONL file with per-row `prompt`, `negative_prompt`, and `size`. See [README](../README.md#input-formats). There is no Hub prompt dataset for image generation.

## What is recorded

- **Per-request records:** each one carries `modality_metrics.images_requested`, `images_returned`, `response_bytes`, and `artifact_<i>_bytes`.
- **Summary:** it holds the shared latency distribution. `--summary-json` writes per-endpoint `images_per_second`.
- **Not a quality score:** there is no image-quality metric. A run measures throughput and latency of whatever the server returned, not whether the images are good.

## Known pitfalls

- **Wrong engine.** Plain vLLM does not serve diffusion models; use vLLM-Omni with `--omni`.
- **Omitted steps.** Leaving out `--num-inference-steps` silently benchmarks the server default (50 for vLLM-Omni Z-Image).
- **Dummy server output.** `dummy-model-server` returns placeholder images. Use it to test plumbing, never as evidence that a real server works ([LIMITATIONS.md](LIMITATIONS.md)).
- **Out of scope.** Agent, KV, multinode, and distributed-KV benchmarks (KYAI, KV, Multinode, DistKV, AgentBench) are Metrum AI Bench Platform features and are not in the OSS binary.

Run `metrum-ai-bench-cli-imagegen --help`, or see [CLI.md](CLI.md), for every flag. [SERVING.md](SERVING.md) covers all modalities.
