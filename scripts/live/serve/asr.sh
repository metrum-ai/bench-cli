#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Metrum AI Bench CLI live smoke: serve openai/whisper-large-v3-turbo for
# POST /v1/audio/transcriptions, 1 GPU.
# Usage: [ASR_STACK=vllm|omni] scripts/live/serve/asr.sh [start|stop|print|logs|sut]
#
# Engine map (docs/SERVING.md): vLLM-Omni is the intended ASR stack, but
# ASR_STACK defaults to `vllm` (vllm/vllm-openai:v0.30.0, vLLM speech-to-text)
# because of a hard blocker found on 2026-10-02. In vllm-omni v0.30.0,
# `--omni` sets the engine's supported tasks to generate/speech only
# (vllm_omni/engine/omni_engine_base.py), so the API server never builds the
# transcription handler and /v1/audio/transcriptions is unavailable under
# --omni for every model. Tracked upstream in
#   https://github.com/vllm-project/vllm-omni/issues/5722 (open 2026-10-02)
# ASR_STACK=omni launches vllm/vllm-omni:v0.30.0 with `vllm serve <model>
# --omni` so the path can be re-validated once that lands; expect the
# transcription route to be missing until then.
#
# --max-model-len 448: Whisper's decoder has 448 positions
# (max_target_positions). vLLM 0.30.0 derives 448 on its own, and the
# official examples pass it explicitly; we pass it so the manifest records it
# and a larger value can never be configured by accident.
#   https://docs.vllm.ai/en/latest/serving/online_serving/speech_to_text/
#   https://github.com/vllm-project/vllm/blob/v0.30.0/examples/generate/multimodal/audio_language_offline.py
# vLLM answers HTTP 400 "Invalid or unsupported audio file" both for bad
# bytes and when an audio decoder library is missing from the image. Stock
# vllm/vllm-openai:v0.30.0 decodes via torchcodec and often logs a soundfile
# ImportError on every upload even when transcriptions succeed. Triage with:
#   docker exec metrum-live-asr python -c "import torchcodec"
# or by sending a real WAV from test-data/asr/. Prefer
#   --media-io-kwargs '{"audio": {"audio_backend": "torchcodec"}}'
# Do not force soundfile on the stock image (it is absent there). See docs/ASR.md.
set -euo pipefail
MODALITY=asr
DEFAULT_MODEL=openai/whisper-large-v3-turbo
MODEL="${MODEL:-${DEFAULT_MODEL}}"
DOCKER_ENV=()
case "${ASR_STACK:-vllm}" in
  vllm)
    IMAGE="${IMAGE:-vllm/vllm-openai:v0.30.0}"
    ENTRYPOINT_CMD=()
    SERVE_ARGS=(--max-model-len 448)
    SOURCES=(
      "https://github.com/vllm-project/vllm/releases/tag/v0.30.0"
      "https://docs.vllm.ai/en/latest/serving/online_serving/speech_to_text/"
      "https://huggingface.co/openai/whisper-large-v3-turbo"
      "https://github.com/vllm-project/vllm-omni/issues/5722"
    )
    SUT_NOTES="vLLM 0.30.0 speech-to-text, whisper-large-v3-turbo, --max-model-len 448 (Whisper decoder positions) per vLLM speech-to-text docs and examples. vLLM-Omni is the intended ASR stack but --omni disables /v1/audio/transcriptions in vllm-omni v0.30.0 (vllm-omni#5722); researched 2026-10-02"
    ;;
  omni)
    IMAGE="${IMAGE:-vllm/vllm-omni:v0.30.0}"
    ENTRYPOINT_CMD=(vllm serve)
    SERVE_ARGS=(--omni --max-model-len 448 --port 8000)
    SOURCES=(
      "https://github.com/vllm-project/vllm-omni/releases/tag/v0.30.0"
      "https://docs.vllm.ai/projects/vllm-omni/en/latest/"
      "https://github.com/vllm-project/vllm-omni/issues/5722"
      "https://huggingface.co/openai/whisper-large-v3-turbo"
    )
    SUT_NOTES="vLLM-Omni 0.30.0 (--omni) serving whisper-large-v3-turbo, --max-model-len 448. Re-validation run for the intended ASR stack; in v0.30.0 --omni does not expose /v1/audio/transcriptions (vllm-omni#5722), so a 404 on that route is the expected result until the fix lands; researched 2026-10-02"
    ;;
  *) echo "error: ASR_STACK must be vllm or omni" >&2; exit 2 ;;
esac
# shellcheck source=scripts/live/serve/common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
serve_main "$@"
