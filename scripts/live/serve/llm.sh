#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Metrum AI Bench CLI live smoke: serve Qwen/Qwen3-8B on vLLM 0.30.0, 1 GPU.
# Usage: scripts/live/serve/llm.sh [start|stop|print|logs]
#
# Flags researched 2026-10-02:
# - vLLM 0.30.0 released 2026-09-22; image vllm/vllm-openai:v0.30.0.
#   https://github.com/vllm-project/vllm/releases/tag/v0.30.0
# - Qwen3 uses --reasoning-parser qwen3 (deepseek_r1 is for QwQ/R1). The
#   model card's --enable-reasoning flag no longer exists in current vLLM.
#   https://docs.vllm.ai/en/latest/features/reasoning_outputs.html
# - Native context 32768 (131072 needs YaRN, not used here).
#   https://huggingface.co/Qwen/Qwen3-8B
# Thinking stays on server-side; local_smoke.sh disables it per request with
# chat_template_kwargs so the 64-token chat-short profile reaches visible text.
# These flags, SOURCES, and SUT_NOTES describe DEFAULT_MODEL only. With
# MODEL=<other>, set SUT_NOTES_OVERRIDE (and usually SERVE_ARGS_OVERRIDE and
# SOURCES_OVERRIDE) from a fresh search; start exits otherwise.
# model.quantization comes from QUANTIZATION, --quantization, or the MODEL
# name (for example Qwen/Qwen3-8B-FP8 records fp8); see common.sh.
set -euo pipefail
MODALITY=llm
IMAGE="${IMAGE:-vllm/vllm-openai:v0.30.0}"
DEFAULT_MODEL=Qwen/Qwen3-8B
MODEL="${MODEL:-${DEFAULT_MODEL}}"
SERVE_ARGS=(--reasoning-parser qwen3 --max-model-len 32768)
DOCKER_ENV=()
ENTRYPOINT_CMD=()
SOURCES=(
  "https://github.com/vllm-project/vllm/releases/tag/v0.30.0"
  "https://docs.vllm.ai/en/latest/features/reasoning_outputs.html"
  "https://huggingface.co/Qwen/Qwen3-8B"
)
SUT_NOTES="vLLM 0.30.0 Qwen3-8B, --reasoning-parser qwen3 --max-model-len 32768 per vLLM reasoning docs and the Qwen3-8B model card (researched 2026-10-02)"
# shellcheck source=scripts/live/serve/common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
serve_main "$@"
