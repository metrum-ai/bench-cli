#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Metrum AI Bench CLI live smoke: serve openai/whisper-large-v3-turbo on
# vLLM 0.30.0, 1 GPU, at POST /v1/audio/transcriptions.
# Usage: scripts/live/serve/asr.sh [start|stop|print|logs]
#
# --max-model-len 448: Whisper's decoder has 448 positions
# (max_target_positions). vLLM 0.30.0 derives 448 on its own, and the
# official examples pass it explicitly; we pass it so the manifest records it
# and a larger value can never be configured by accident.
#   https://docs.vllm.ai/en/latest/serving/online_serving/speech_to_text/
#   https://github.com/vllm-project/vllm/blob/v0.30.0/examples/generate/multimodal/audio_language_offline.py
# vLLM answers HTTP 400 "Invalid or unsupported audio file" both for bad
# bytes and when an audio decoder library is missing from the image. If
# valid WAVs from test-data/asr/ get that error, check the container with
#   docker exec metrum-live-asr python -c "import soundfile, av"
# and, if needed, add
#   --media-io-kwargs '{"audio": {"audio_backend": "soundfile"}}'
# (docs/features/multimodal_inputs.md in vLLM v0.30.0).
set -euo pipefail
MODALITY=asr
IMAGE="${IMAGE:-vllm/vllm-openai:v0.30.0}"
MODEL="${MODEL:-openai/whisper-large-v3-turbo}"
SERVE_ARGS=(--max-model-len 448)
DOCKER_ENV=()
ENTRYPOINT_CMD=()
SOURCES=(
  "https://github.com/vllm-project/vllm/releases/tag/v0.30.0"
  "https://docs.vllm.ai/en/latest/serving/online_serving/speech_to_text/"
  "https://huggingface.co/openai/whisper-large-v3-turbo"
)
SUT_NOTES="vLLM 0.30.0 whisper-large-v3-turbo, --max-model-len 448 (Whisper decoder positions) per vLLM speech-to-text docs and examples (researched 2026-10-02)"
# shellcheck source=scripts/live/serve/common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
serve_main "$@"
