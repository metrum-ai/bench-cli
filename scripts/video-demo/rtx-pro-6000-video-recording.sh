#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# RTX PRO 6000 recording pass for the Bench CLI product video (Scenes 2-8 of
# the approved narration plan). Run this on the RTX PRO 6000 host while
# screen-recording the terminal, one workload block at a time. Repeat the
# same commands with the H100 endpoint (swap $URL / $SUT_FILE) to get the
# comparison footage for Scene 8.
#
# Fill in the four environment variables below before running. Nothing here
# invents numbers: every flag matches metrum-ai-bench's documented CLI
# (docs/CLI.md) and the workload parameters in your test matrix CSV.

set -euo pipefail

# ---- Fill these in for the RTX PRO 6000 host ------------------------------
URL="${URL:-http://REPLACE-ME:8000}"          # base OpenAI-compatible URL, no trailing /v1/... path segment for imagegen
CHAT_PATH="${CHAT_PATH:-$URL/v1/chat/completions}"
TRANSCRIBE_PATH="${TRANSCRIBE_PATH:-$URL/v1/audio/transcriptions}"
API_KEY="${API_KEY:-REPLACE-ME}"
OUT_DIR="${OUT_DIR:-./bench-video-runs/rtx-pro-6000}"
SUT_FILE="${SUT_FILE:-./sut-rtx-pro-6000.json}" # {"accelerator":"NVIDIA RTX PRO 6000", ...} — see docs/OUTPUT_SCHEMA.md
# ----------------------------------------------------------------------------

mkdir -p "$OUT_DIR"

echo "### Scene 2/3 — help + selftest ###"
metrum-ai-bench --help
metrum-ai-bench selftest

echo "### Scene 4 — LLM (Qwen/Qwen3.8-27B-FP8, ISL/OSL 1024x1024) ###"
# Build a real ISL/OSL-matched prompt mix from the published prompt library
# instead of hand-written filler text (see README "Prompt-library mix").
metrum-ai-bench-prompts \
  --config sample --count 128 --seed 42 \
  --isl-target 1024 --isl-unit tokens --isl-stat mean --isl-tolerance 64 \
  --osl-target 1024 --osl-unit tokens --osl-stat mean --osl-tolerance 64 \
  --output "$OUT_DIR/llm-mix.jsonl" --report "$OUT_DIR/llm-mix-report.json"

MAX_TOKENS=$(jq .recommended_max_tokens "$OUT_DIR/llm-mix-report.json")

for CONCURRENCY in 1 32 64 128; do
  metrum-ai-bench llm -- \
    --url "$CHAT_PATH" --api-key "$API_KEY" \
    --scenario "rtx-pro-6000-llm-c${CONCURRENCY}" \
    --num-requests "$(jq .selected_count "$OUT_DIR/llm-mix-report.json")" \
    --concurrency "$CONCURRENCY" --warmup-requests 0 \
    --prompts "$OUT_DIR/llm-mix.jsonl" \
    --mode chat --streaming \
    --model "Qwen/Qwen3.8-27B-FP8" \
    --max-tokens "$MAX_TOKENS" \
    --sut "$SUT_FILE" --require-sut \
    --data-log "$OUT_DIR/llm-c${CONCURRENCY}.jsonl"
done

echo "### Scene 5 — VLM (same model + image prompts, ISL/OSL 1024x1024) ###"
# Adapt the same text mix into VLM prompt format (prompt + image_url per line).
# Replace REPLACE-ME.png with your real evaluation image(s).
jq -c '{prompt: .prompt, image_url: "REPLACE-ME.png"}' "$OUT_DIR/llm-mix.jsonl" \
  > "$OUT_DIR/vlm-mix.jsonl"

for CONCURRENCY in 1 8 16 32; do
  metrum-ai-bench vlm -- \
    --url "$CHAT_PATH" --api-key "$API_KEY" \
    --scenario "rtx-pro-6000-vlm-c${CONCURRENCY}" \
    --num-requests 64 --concurrency "$CONCURRENCY" \
    --prompts "$OUT_DIR/vlm-mix.jsonl" \
    --model "Qwen/Qwen3.8-27B-FP8" --max-tokens "$MAX_TOKENS" \
    --sut "$SUT_FILE" --require-sut \
    --data-log "$OUT_DIR/vlm-c${CONCURRENCY}.jsonl"
done

echo "### Scene 6 — ASR (openai/whisper-large-v3) ###"
# Point AUDIO_DIR at a folder of real audio files before recording; the
# shipped test-data/dummy.mp3 fixture is a placeholder only.
AUDIO_DIR="${AUDIO_DIR:-./test-data}"
python3 - "$AUDIO_DIR" > "$OUT_DIR/asr-input.jsonl" <<'PY'
import json, sys, pathlib
d = pathlib.Path(sys.argv[1])
for i, f in enumerate(sorted(d.glob("*.mp3"))):
    print(json.dumps({"id": f"sample-{i}", "path": str(f), "format": "mp3"}))
PY

for CONCURRENCY in 1 32 64 128; do
  metrum-ai-bench asr -- \
    --url "$TRANSCRIBE_PATH" --api-key "$API_KEY" \
    --scenario "rtx-pro-6000-asr-c${CONCURRENCY}" \
    --num-requests 64 --concurrency "$CONCURRENCY" \
    --input "$OUT_DIR/asr-input.jsonl" \
    --model "openai/whisper-large-v3" \
    --sut "$SUT_FILE" --require-sut \
    --data-log "$OUT_DIR/asr-c${CONCURRENCY}.jsonl"
done

echo "### Scene 7 — Image generation (Nemotron-3.5-Lightning-30B-A3B-NVFP4, 4K) ###"
for CONCURRENCY in 1 2 4 8; do
  metrum-ai-bench imagegen -- \
    --url "$URL/v1" --api-key "$API_KEY" \
    --scenario "rtx-pro-6000-imagegen-c${CONCURRENCY}" \
    --model "nvidia/NVIDIA-Nemotron-3.5-Lightning-30B-A3B-NVFP4" \
    --num-requests 16 --concurrency "$CONCURRENCY" \
    --prompt "a red cube on a table, studio lighting" \
    --size 3840x2160 \
    --sut "$SUT_FILE" --require-sut \
    --data-log "$OUT_DIR/imagegen-c${CONCURRENCY}.jsonl"
done

echo "### Scene 8 — HTML report for one representative run ###"
metrum-ai-bench-strategic \
  --url "$CHAT_PATH" --model "Qwen/Qwen3.8-27B-FP8" \
  --sweep 1,32,64,128 --sweep-by concurrency --requests-per-stage 64 \
  --html "$OUT_DIR/report-rtx-pro-6000.html" --csv "$OUT_DIR/report-rtx-pro-6000.csv"

echo "Done. Outputs are in $OUT_DIR — repeat this whole script against the H100 endpoint (different \$URL and \$SUT_FILE) for the Scene 8 side-by-side."
