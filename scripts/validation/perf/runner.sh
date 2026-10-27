#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Standard perf-validation runner (issue #333). Runs the spec.yaml point grid
# for ONE model on ONE GPU box, interleaving baseline and candidate builds per
# point, and writes one directory per point with the raw request log, NDJSON,
# power CSVs and a point.json. Self-contained: no MLPerf client needed.
#
# Usage:
#   runner.sh [--plan] <model-tag> <baseline-binary> <candidate-binary> <out-root> [passes]
# Example:
#   runner.sh q38-27b \
#     /opt/bench-rel/v1.5.4/bin/metrum-ai-bench-cli-llm \
#     /opt/src/bench-cli/target/release/metrum-ai-bench-cli-llm \
#     ~/perf-runs/q38-27b 1
#
# --plan: dry-run. Resolves the model and pins, prints the serve command and
# the full point grid it would run (with the binary and dataset checks), then
# exits 0 without touching docker, the GPU or the bench binaries. Needs no
# DATASETS_DIR/HF_TOKEN — CI-safe on a lint-only host.
#
# Required env: HF_TOKEN (from a 0600 file, never argv), DATASETS_DIR with
# W1.jsonl and W2.jsonl. Optional: VLLM_IMAGE (default: spec digest),
# TELEMETRY_YAML, ALL_SMI_BIN (enables exporter + power sampling).
set -euo pipefail

PLAN=0
if [[ "${1:-}" == "--plan" ]]; then PLAN=1; shift; fi
TAG=${1:-}; BASE_BIN=${2:-}; CAND_BIN=${3:-}; OUT=${4:-}; PASSES=${5:-1}
if [[ $# -lt 4 || -z "$TAG" ]]; then
  echo "usage: runner.sh [--plan] <model-tag> <baseline-binary> <candidate-binary> <out-root> [passes]" >&2
  exit 2
fi
DATASETS_DIR=${DATASETS_DIR:-}
VLLM_IMAGE=${VLLM_IMAGE:-vllm/vllm-openai@sha256:befccf0e7e4089aa23926bb6f887a85c0d46c2a99e6de5c5ffdf5cb7957c5b34}
WARMUP=${WARMUP:-30}; DURATION=${DURATION:-180}

case $TAG in
  q38-27b)       MODEL=Qwen/Qwen3.8-27B;        REV=1d4bf0f2ff6012fd82039f2fa52739d0dd7c60c0; EXTRA=();;
  q38-27b-nvfp4) MODEL=nvidia/Qwen3.8-27B-NVFP4; REV=482ca0f3832238542f8f5295dde86b5f22711d80
                 EXTRA=(--compilation-config '{"cudagraph_mode":"PIECEWISE"}');;
  *) echo "unknown model tag $TAG" >&2; exit 2;;
esac

if [[ $PLAN -eq 1 ]]; then
  echo "perf-validation runner PLAN (no docker, no GPU, no requests)"
  echo "  model:      $MODEL @ $REV"
  echo "  image:      $VLLM_IMAGE"
  echo "  out-root:   $OUT"
  echo "  passes:     $PASSES"
  echo "  baseline:   $BASE_BIN $([[ -x $BASE_BIN ]] && echo "(exists)" || echo "(MISSING)")"
  echo "  candidate:  $CAND_BIN $([[ -x $CAND_BIN ]] && echo "(exists)" || echo "(MISSING)")"
  echo "  datasets:   ${DATASETS_DIR:-<unset>}" \
       "W1=$([[ -f ${DATASETS_DIR:-/nonexistent}/W1.jsonl ]] && echo ok || echo MISSING)" \
       "W2=$([[ -f ${DATASETS_DIR:-/nonexistent}/W2.jsonl ]] && echo ok || echo MISSING)"
  echo "  warmup/measure: ${WARMUP}s / ${DURATION}s"
  echo "  serve:"
  echo "    docker run -d --name perfval-vllm --gpus all --ipc=host \\"
  echo "      -p 127.0.0.1:8000:8000 -e HF_TOKEN \\"
  echo "      -v \$HOME/.cache/huggingface:/root/.cache/huggingface \\"
  echo "      $VLLM_IMAGE --model $MODEL --revision $REV \\"
  echo "      --max-model-len 16384 --max-num-seqs 128 --gpu-memory-utilization 0.90 \\"
  echo "      --default-chat-template-kwargs '{\"enable_thinking\": false}' ${EXTRA[*]:-} \\"
  echo "      --no-enable-prefix-caching"
  echo "  grid (per pass, interleaved baseline/candidate; idempotent per point):"
  for ((p=1;p<=PASSES;p++)); do
    for w in W1 W2; do
      for c in 1 8 32; do
        for label in bench-rel bench-src; do
          echo "    pass$p $label $w c$c -> $OUT/$label/$p/$w/c$c (warmup ${WARMUP}s + measure ${DURATION}s)"
        done
      done
    done
  done
  echo "RUNNER PLAN OK (no side effects)"
  exit 0
fi

: "${DATASETS_DIR:?set DATASETS_DIR with W1.jsonl and W2.jsonl}"
mkdir -p "$OUT"

serve() {
  docker rm -f perfval-vllm >/dev/null 2>&1 || true
  docker run -d --name perfval-vllm --gpus all --ipc=host \
    -p 127.0.0.1:8000:8000 -e HF_TOKEN \
    -v "$HOME/.cache/huggingface:/root/.cache/huggingface" \
    "$VLLM_IMAGE" --model "$MODEL" --revision "$REV" \
    --max-model-len 16384 --max-num-seqs 128 --gpu-memory-utilization 0.90 \
    --default-chat-template-kwargs '{"enable_thinking": false}' \
    "${EXTRA[@]}" --no-enable-prefix-caching > "$OUT/container.id"
  for ((i=0;i<1800;i++)); do
    docker logs perfval-vllm > "$OUT/vllm.log" 2>&1 || true
    curl -fsS http://127.0.0.1:8000/v1/models > "$OUT/models.json" 2>/dev/null && return 0
    sleep 2
  done
  echo "server did not become ready" >&2; return 1
}

run_one() { # binary workload concurrency pass tool-label out-dir duration
  local bin=$1 w=$2 c=$3 pass=$4 label=$5 out=$6 dur=$7
  local mt=512; [[ $w == W1 ]] && mt=128
  mkdir -p "$out"
  if command -v nvidia-smi >/dev/null; then
    nvidia-smi --query-gpu=timestamp,power.draw,power.limit,temperature.gpu,utilization.gpu,memory.used \
      --format=csv -lms 1000 > "$out/nvidia.csv" 2>/dev/null &
    local npid=$!
  fi
  local args=(
    --scenario "$w" --url http://127.0.0.1:8000/v1/chat/completions --api-key dummy
    --num-requests 1000000 --concurrency "$c"
    --prompts "$DATASETS_DIR/$w.jsonl" --mode chat --streaming --model "$MODEL"
    --data-log "$out/requests.jsonl" --max-tokens "$mt"
    --temperature 0 --seed 42 --extra-body-json '{"seed":42}'
    --stop-after-seconds "$dur" --debug-log "$out/debug.log" --error-log "$out/error.log"
  )
  if [[ -n "${TELEMETRY_YAML:-}" ]]; then
    args+=(--ndjson "$out/run.ndjson" --telemetry "$TELEMETRY_YAML" --require-telemetry)
  fi
  echo "{\"tool\":\"$label\",\"workload\":\"$w\",\"concurrency\":$c,\"pass\":\"$pass\",\"start\":$(date +%s.%N)}" > "$out/point.json"
  local rc=0
  "$bin" "${args[@]}" > "$out/stdout.log" 2>&1 || rc=$?
  [[ -n "${npid:-}" ]] && kill "$npid" 2>/dev/null || true
  python3 - "$out/point.json" <<'PY'
import json, sys, time
p = sys.argv[1]
m = json.load(open(p)); m["stop"] = time.time(); json.dump(m, open(p, "w"))
PY
  [[ $rc -eq 0 ]] && touch "$out/DONE"
  return $rc
}

serve
for ((p=1;p<=PASSES;p++)); do
  for w in W1 W2; do
    for c in 1 8 32; do
      for pair in "bench-rel $BASE_BIN" "bench-src $CAND_BIN"; do
        label=${pair%% *}; bin=${pair#* }
        out="$OUT/$label/$p/$w/c$c"
        [[ -f "$out/DONE" ]] && { echo "skip $label/$p/$w/c$c"; continue; }
        echo "=== $label pass$p $w c$c $(date -u +%FT%TZ) ==="
        [[ $WARMUP -gt 0 ]] && run_one "$bin" "$w" "$c" "$p" "$label" "$out/warmup" "$WARMUP"
        run_one "$bin" "$w" "$c" "$p" "$label" "$out" "$DURATION" \
          && echo "OK $label/$p/$w/c$c" || echo "FAIL $label/$p/$w/c$c"
      done
    done
  done
done
echo "RUNNER $TAG DONE $(date -u +%FT%TZ)"
