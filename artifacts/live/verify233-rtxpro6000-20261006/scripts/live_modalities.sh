#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
# Metrum AI Bench CLI: epic #184 live validation, VLM/ASR/imagegen lanes (runs on the GPU host).
# Usage: live_modalities.sh vlm|asr|imagegen
set -uo pipefail
m="$1"
R=/opt/metrum-bench
B=$R/bin
OUT=$R/live-results/verify233-$m
rm -rf "$OUT"; mkdir -p "$OUT"
cd "$R"
log() { echo "# $(date -u +%H:%M:%SZ) $*"; }

# Stop whatever is serving on :8000 (previous lane), then start this lane's launcher.
for l in llm vlm asr imagegen; do bash scripts/live/serve/$l.sh stop >/dev/null 2>&1 || true; done
case "$m" in
  vlm)
    export IMAGE=vllm/vllm-openai:v0.31.0
    export SOURCES_OVERRIDE="https://github.com/vllm-project/vllm/releases/tag/v0.31.0 https://huggingface.co/Qwen/Qwen3-VL-8B-Instruct"
    export SUT_NOTES_OVERRIDE="vLLM 0.31.0 (2026-10-04) Qwen3-VL-8B-Instruct with the launcher's Qwen3-VL recipe flags (researched 2026-10-05); epic #184 live validation on 1x H100 PCIe"
    EP=http://127.0.0.1:8000/v1/chat/completions; MODEL=Qwen/Qwen3-VL-8B-Instruct; ENGINE_RE='^vllm:' ;;
  asr)
    export ASR_STACK=vllm IMAGE=vllm/vllm-openai:v0.31.0
    export SOURCES_OVERRIDE="https://github.com/vllm-project/vllm/releases/tag/v0.31.0 https://docs.vllm.ai/en/latest/serving/openai_compatible_server.html https://huggingface.co/openai/whisper-large-v3-turbo"
    export SUT_NOTES_OVERRIDE="vLLM 0.31.0 (2026-10-04) speech-to-text, whisper-large-v3-turbo, --max-model-len 448; vLLM-Omni still blocked by vllm-omni#5722 (researched 2026-10-05); epic #184 live validation on 1x H100 PCIe"
    EP=http://127.0.0.1:8000/v1/audio/transcriptions; MODEL=openai/whisper-large-v3-turbo; ENGINE_RE='^vllm:' ;;
  imagegen)
    EP=http://127.0.0.1:8000/v1/images/generations; MODEL=Tongyi-MAI/Z-Image-Turbo; ENGINE_RE='^vllm_omni:' ;;
esac
log "starting $m launcher"
bash scripts/live/serve/$m.sh start > "$OUT/serve.log" 2>&1 &
for i in $(seq 1 120); do curl -fsS -m 3 http://127.0.0.1:8000/v1/models >/dev/null 2>&1 && break; sleep 10; done
curl -fsS http://127.0.0.1:8000/v1/models > "$OUT/models.json" || { log "server not ready"; tail -40 "$OUT/serve.log"; docker logs --tail 60 metrum-live-$m 2>&1 | tail -60; exit 2; }
log "server ready"
SUT="$OUT/sut.json"
cp "$R/live-results/serve-$m/sut.json" "$SUT"
jq '.extra |= with_entries(.value |= tostring)' "$SUT" > "$SUT.tmp" && mv "$SUT.tmp" "$SUT"

# Telemetry: all-smi fork plus the engine page (allow_empty so a renamed engine family does not abort the run).
python3 - "$OUT/telemetry.yaml" "$ENGINE_RE" <<'PY'
import sys, yaml
a = yaml.safe_load(open("docs/telemetry/examples/all-smi.yaml"))
srcs = [s for s in a["sources"] if s["name"] != "vllm"]
srcs.append({"name": "engine", "url": "http://127.0.0.1:8000/metrics", "interval_ms": 1000,
             "include": [sys.argv[2]], "allow_empty": True})
a["sources"] = srcs
yaml.safe_dump(a, open(sys.argv[1], "w"), sort_keys=False)
PY
tele=(--telemetry "$OUT/telemetry.yaml")

log "single run: $m with telemetry and ndjson"
case "$m" in
  vlm)
    "$B/metrum-ai-bench-cli-vlm" --url "$EP" --api-key dummy --model "$MODEL" --scenario epic184-vlm --streaming \
      --prompts test-data/vlm/prompts.jsonl --num-requests 64 --warmup-requests 8 --concurrency 8 --max-tokens 128 \
      --sut "$SUT" --require-sut "${tele[@]}" --ndjson "$OUT/run.ndjson" --data-log "$OUT/run.jsonl" > "$OUT/run.stdout" 2> "$OUT/run.stderr"
    echo "vlm exit $?" ;;
  asr)
    "$B/metrum-ai-bench-cli-asr" --url "$EP" --api-key dummy --model "$MODEL" --scenario epic184-asr \
      --input test-data/asr/input.jsonl --ground-truth test-data/asr/truth.jsonl --num-requests 96 --concurrency 8 \
      --sut "$SUT" --require-sut "${tele[@]}" --ndjson "$OUT/run.ndjson" --data-log "$OUT/run.jsonl" > "$OUT/run.stdout" 2> "$OUT/run.stderr"
    echo "asr exit $?" ;;
  imagegen)
    "$B/metrum-ai-bench-cli-imagegen" --url "$EP" --api-key dummy --model "$MODEL" --scenario epic184-imagegen \
      --prompt "a lighthouse on a rocky coast at dusk, watercolor" --num-requests 8 --concurrency 1 \
      --num-inference-steps 9 --guidance-scale 0.0 --artifact-dir "$OUT/images" \
      --sut "$SUT" --require-sut "${tele[@]}" --ndjson "$OUT/run.ndjson" --data-log "$OUT/run.jsonl" > "$OUT/run.stdout" 2> "$OUT/run.stderr"
    echo "imagegen exit $?" ;;
esac

if [ "$m" = vlm ]; then
  log "#242 large-image VLM run (2048x2048 PNG)"
  python3 - "$OUT/big.png" <<'PY'
import sys, zlib, struct
w = h = 2048
rows = []
for y in range(h):
    row = bytearray([0])
    for x in range(w):
        row += bytes(((x + y) & 255, (x * 3) & 255, (y * 5 + x) & 255))
    rows.append(bytes(row))
data = zlib.compress(b"".join(rows), 6)
def chunk(t, d): return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", data) + chunk(b"IEND", b"")
open(sys.argv[1], "wb").write(png)
PY
  printf '{"prompt":"Describe this image in one sentence.","image_url":"%s"}\n' "$OUT/big.png" > "$OUT/big-prompts.jsonl"
  "$B/metrum-ai-bench-cli-vlm" --url "$EP" --api-key dummy --model "$MODEL" --scenario verify-242-big --streaming \
    --prompts "$OUT/big-prompts.jsonl" --num-requests 16 --warmup-requests 2 --concurrency 4 --max-tokens 64 \
    --data-log "$OUT/big.jsonl" --ndjson "$OUT/big.ndjson" > "$OUT/big.stdout" 2> "$OUT/big.stderr"
  echo "vlm big exit $?"; ls -la "$OUT/big.png"
fi
log "strategic sweep: --kind $m, 5 stages"
case "$m" in
  vlm)
    "$B/metrum-ai-bench-cli-strategic" --url "$EP" --api-key dummy --model "$MODEL" --kind vlm --streaming \
      --prompt "Name the shapes, their colors, and the word in this image." --image test-data/vlm/shapes-512.png \
      --max-tokens 128 --sweep-by concurrency --sweep 1,2,4,8,16 --requests-per-stage 32 --warmup-requests 4 \
      "${tele[@]}" --ndjson "$OUT/sweep.ndjson" --csv "$OUT/sweep.csv" > "$OUT/sweep.stdout.json" 2> "$OUT/sweep.stderr"
    echo "sweep exit $?" ;;
  asr)
    "$B/metrum-ai-bench-cli-strategic" --url "$EP" --api-key dummy --model "$MODEL" --kind asr \
      --audio-samples test-data/asr/input.jsonl --ground-truth test-data/asr/truth.jsonl \
      --sweep-by concurrency --sweep 1,2,4,8,16 --requests-per-stage 48 --warmup-requests 4 \
      "${tele[@]}" --ndjson "$OUT/sweep.ndjson" --csv "$OUT/sweep.csv" > "$OUT/sweep.stdout.json" 2> "$OUT/sweep.stderr"
    echo "sweep exit $?" ;;
  imagegen)
    "$B/metrum-ai-bench-cli-strategic" --url "$EP" --api-key dummy --model "$MODEL" --kind imagegen \
      --prompt "a lighthouse on a rocky coast at dusk, watercolor" --image-size 1024x1024 \
      --extra-body-json '{"num_inference_steps":9,"guidance_scale":0.0}' \
      --sweep-by concurrency --sweep 1,2,3,4,5 --requests-per-stage 6 --warmup-requests 1 \
      "${tele[@]}" --ndjson "$OUT/sweep.ndjson" --csv "$OUT/sweep.csv" > "$OUT/sweep.stdout.json" 2> "$OUT/sweep.stderr"
    echo "sweep exit $?" ;;
esac
log "done $m"
ls -la "$OUT"
