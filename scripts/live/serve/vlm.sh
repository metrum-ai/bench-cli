#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Metrum AI Bench CLI live smoke: serve Qwen/Qwen3-VL-8B-Instruct on vLLM
# 0.30.0, 1 GPU.
# Usage: scripts/live/serve/vlm.sh [start|stop|print|logs|sut]
#
# Flags researched 2026-10-02 from the vLLM Qwen3-VL recipe (written for the
# 235B model on 8 GPUs; the single-GPU 8B values here are extrapolated):
#   https://docs.vllm.ai/projects/recipes/en/latest/Qwen/Qwen3-VL.html
# - --limit-mm-per-prompt.video 0: image-only, skips video profiling.
# - --max-model-len 128000 instead of the 262144 default.
# - --async-scheduling and OMP_NUM_THREADS=1.
# - --mm-processor-cache-gb 0 because benchmark images are not reused.
# Model card: https://huggingface.co/Qwen/Qwen3-VL-8B-Instruct
# These flags, SOURCES, and SUT_NOTES describe DEFAULT_MODEL on DEFAULT_IMAGE
# only. With MODEL=<other>, IMAGE=<other>, or a different SERVE_ARGS_OVERRIDE,
# set SUT_NOTES_OVERRIDE and SOURCES_OVERRIDE from a fresh search; start and
# sut exit otherwise.
set -euo pipefail
MODALITY=vlm
DEFAULT_IMAGE=vllm/vllm-openai:v0.30.0
IMAGE="${IMAGE:-${DEFAULT_IMAGE}}"
DEFAULT_MODEL=Qwen/Qwen3-VL-8B-Instruct
MODEL="${MODEL:-${DEFAULT_MODEL}}"
SERVE_ARGS=(--max-model-len 128000 --limit-mm-per-prompt.video 0 --async-scheduling --mm-processor-cache-gb 0)
DOCKER_ENV=(OMP_NUM_THREADS=1)
ENTRYPOINT_CMD=()
SOURCES=(
  "https://github.com/vllm-project/vllm/releases/tag/v0.30.0"
  "https://docs.vllm.ai/projects/recipes/en/latest/Qwen/Qwen3-VL.html"
  "https://huggingface.co/Qwen/Qwen3-VL-8B-Instruct"
)
SUT_NOTES="vLLM 0.30.0 Qwen3-VL-8B-Instruct, flags from the vLLM Qwen3-VL recipe (235B recipe extrapolated to 8B on 1 GPU; researched 2026-10-02)"
# shellcheck source=scripts/live/serve/common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
serve_main "$@"
