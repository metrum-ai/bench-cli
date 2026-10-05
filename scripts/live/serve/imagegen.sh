#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Metrum AI Bench CLI live smoke: serve Tongyi-MAI/Z-Image-Turbo on
# vllm-omni 0.30.0, 1 GPU, at POST /v1/images/generations (b64_json).
# Usage: scripts/live/serve/imagegen.sh [start|stop|print|logs|sut]
#
# Researched 2026-10-02:
# - vllm-omni 0.30.0 (2026-09-25), image vllm/vllm-omni:v0.30.0. The image
#   has no default entrypoint, so the command is `vllm serve <model> --omni`.
#   https://github.com/vllm-project/vllm-omni/blob/v0.30.0/docs/getting_started/installation/gpu/cuda.inc.md
# - Image API fields (prompt, n, size, response_format, num_inference_steps,
#   guidance_scale, seed):
#   https://docs.vllm.ai/projects/vllm-omni/en/latest/serving/image_generation_api/
# - Z-Image-Turbo: 9 inference steps, guidance 0.0, 1024x1024.
#   https://huggingface.co/Tongyi-MAI/Z-Image-Turbo
# The server defaults to 50 steps when the request omits
# num_inference_steps, so local_smoke.sh always sends 9 steps and guidance 0.
# No vendor H100 serving recipe exists for Z-Image; the vllm-omni recipe
# covers offline Intel XPU only.
set -euo pipefail
MODALITY=imagegen
DEFAULT_IMAGE=vllm/vllm-omni:v0.30.0
IMAGE="${IMAGE:-${DEFAULT_IMAGE}}"
DEFAULT_MODEL=Tongyi-MAI/Z-Image-Turbo
MODEL="${MODEL:-${DEFAULT_MODEL}}"
SERVE_ARGS=(--omni --port 8000)
DOCKER_ENV=()
ENTRYPOINT_CMD=(vllm serve)
SOURCES=(
  "https://github.com/vllm-project/vllm-omni/blob/v0.30.0/docs/getting_started/installation/gpu/cuda.inc.md"
  "https://docs.vllm.ai/projects/vllm-omni/en/latest/serving/image_generation_api/"
  "https://huggingface.co/Tongyi-MAI/Z-Image-Turbo"
)
SUT_NOTES="vllm-omni 0.30.0 Z-Image-Turbo via /v1/images/generations; requests send num_inference_steps=9 guidance_scale=0.0 size=1024x1024 per the model card (server default is 50 steps; researched 2026-10-02)"
# shellcheck source=scripts/live/serve/common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
serve_main "$@"
