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
# Required env: DATASETS_DIR with W1.jsonl and W2.jsonl and exactly one
# CUDA_VISIBLE_DEVICES selection. PERF_OFFLINE=1 uses a prepared hub cache and
# already-present image. Optional: VLLM_IMAGE and TELEMETRY_YAML.
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
  echo "    docker run -d --name <owned-unique-name> --gpus device=<selected-gpu> --ipc=host \\"
  echo "      -p 127.0.0.1:8000:8000 \\"
  if [[ "${PERF_OFFLINE:-0}" == 1 ]]; then
    echo "      --pull=never -e HF_HUB_OFFLINE=1 \\"
  else
    echo "      <HF_TOKEN passed by environment only if present> \\"
  fi
  echo "      -v <hub-cache>:/root/.cache/huggingface/hub \\"
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
: "${CUDA_VISIBLE_DEVICES:?select one CUDA_VISIBLE_DEVICES GPU}"
[[ "$CUDA_VISIBLE_DEVICES" =~ ^([0-9]+|GPU-[0-9a-fA-F-]+)$ ]] || { echo 'select exactly one GPU' >&2; exit 2; }
mkdir -p "$OUT"
OWNER="${PERF_OWNER:-$(python3 -c 'import secrets; print(secrets.token_hex(16))')}"
[[ "$OWNER" =~ ^[0-9a-f]{32}$ ]] || { echo 'invalid performance owner token' >&2; exit 2; }
CONTAINER="perfval-${OWNER}"
STARTED=0 SAMPLE_PID=''
# Called by the EXIT trap.
# shellcheck disable=SC2329
cleanup() {
  local rc=$1 owner remaining
  trap - EXIT INT TERM
  set +e
  if [[ -n "$SAMPLE_PID" ]]; then kill "$SAMPLE_PID" 2>/dev/null; wait "$SAMPLE_PID" 2>/dev/null; fi
  if ((STARTED)); then
    remaining="$(docker ps -a --filter "label=metrum.cert.owner=$OWNER" --format '{{.ID}}')" || rc=1
    if [[ -n "$remaining" ]]; then
      owner="$(docker inspect --format '{{index .Config.Labels "metrum.cert.owner"}}' "$CONTAINER" 2>/dev/null)" || owner=''
      if [[ "$owner" != "$OWNER" ]]; then
        echo 'container ownership mismatch; refusing removal' >&2; rc=1
      else
        docker logs "$CONTAINER" > "$OUT/vllm.log" 2>&1
        docker rm -f "$CONTAINER" > "$OUT/container-stop.txt" 2>&1 || rc=1
        remaining="$(docker ps -a --filter "label=metrum.cert.owner=$OWNER" --format '{{.ID}}')" || rc=1
        [[ -z "$remaining" ]] || rc=1
      fi
    fi
  fi
  exit "$rc"
}
trap 'cleanup "$?"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

serve() {
  local hub="${CERT_HF_HUB_CACHE:-$HOME/.cache/huggingface/hub}"
  local -a offline=() token=()
  if [[ "${PERF_OFFLINE:-0}" == 1 ]]; then
    [[ -d "$hub" ]] || { echo 'offline hub cache is missing' >&2; return 1; }
    offline=(--pull=never -e HF_HUB_OFFLINE=1)
  else
    mkdir -p "$hub"
    [[ -z "${HF_TOKEN:-}" ]] || token=(-e HF_TOKEN)
  fi
  hub="$(realpath "$hub")"
  [[ "$hub" != *:* && "$hub" != *$'\n'* ]] || { echo 'invalid hub cache path' >&2; return 1; }
  STARTED=1
  docker run -d --name "$CONTAINER" --label "metrum.cert.owner=$OWNER" --gpus "device=$CUDA_VISIBLE_DEVICES" --ipc=host \
    -p 127.0.0.1:8000:8000 "${offline[@]}" "${token[@]}" \
    -v "$hub:/root/.cache/huggingface/hub" \
    "$VLLM_IMAGE" --model "$MODEL" --revision "$REV" \
    --max-model-len 16384 --max-num-seqs 128 --gpu-memory-utilization 0.90 \
    --default-chat-template-kwargs '{"enable_thinking": false}' \
    "${EXTRA[@]}" --no-enable-prefix-caching > "$OUT/container.id"
  for ((i=0;i<1800;i++)); do
    docker logs "$CONTAINER" > "$OUT/vllm.log" 2>&1 || true
    curl -fsS http://127.0.0.1:8000/v1/models > "$OUT/models.json" 2>/dev/null && return 0
    docker inspect --format '{{.State.Running}}' "$CONTAINER" | grep -qx true || return 1
    sleep 2
  done
  echo "server did not become ready" >&2; return 1
}

run_one() { # binary workload concurrency pass tool-label out-dir duration
  local bin=$1 w=$2 c=$3 pass=$4 label=$5 out=$6 dur=$7
  local mt=512; [[ $w == W1 ]] && mt=128
  mkdir -p "$out" || return 1
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
  if [[ "${CERT_PERF_SEQUENCE:-0}" == 1 && "$out" != */warmup ]]; then
    args+=(--prompt-sequence-log "$out/prompt-sequence.jsonl")
  fi
  echo "{\"tool\":\"$label\",\"workload\":\"$w\",\"concurrency\":$c,\"pass\":\"$pass\",\"start\":$(date +%s.%N)}" > "$out/point.json" || return 1
  if ! python3 - "$out/command.json" "$bin" "$DATASETS_DIR/$w.jsonl" "${args[@]}" <<'PY'
import hashlib, json, pathlib, sys
out, binary, prompt, *arguments = sys.argv[1:]
def digest(path):
    h = hashlib.sha256()
    with open(path, 'rb') as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b''): h.update(chunk)
    return h.hexdigest()
pathlib.Path(out).write_text(json.dumps({'argv': [binary, *arguments], 'binary_sha256': digest(binary),
    'prompt_sha256': digest(prompt)}, indent=2) + '\n', encoding='utf-8')
PY
  then
    return 1
  fi
  if command -v nvidia-smi >/dev/null; then
    TZ=UTC nvidia-smi --id="$CUDA_VISIBLE_DEVICES" --query-gpu=timestamp,uuid,power.draw,power.limit,temperature.gpu,utilization.gpu,memory.used \
      --format=csv -lms 1000 > "$out/nvidia.csv" 2>/dev/null &
    SAMPLE_PID=$!
  fi
  local rc=0
  "$bin" "${args[@]}" > "$out/stdout.log" 2>&1 || rc=$?
  if [[ -n "$SAMPLE_PID" ]]; then kill "$SAMPLE_PID" 2>/dev/null || true; wait "$SAMPLE_PID" 2>/dev/null || true; SAMPLE_PID=''; fi
  python3 - "$out/point.json" "$out/command.json" "$bin" "$DATASETS_DIR/$w.jsonl" <<'PY' || rc=1
import hashlib, json, pathlib, sys, time
point, command, binary, prompt = map(pathlib.Path, sys.argv[1:])
def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b''): h.update(chunk)
    return h.hexdigest()
m = json.loads(point.read_text()); m['stop'] = time.time(); point.write_text(json.dumps(m))
c = json.loads(command.read_text()); c['binary_sha256_end'] = digest(binary); c['prompt_sha256_end'] = digest(prompt)
command.write_text(json.dumps(c, indent=2) + '\n')
if c['binary_sha256_end'] != c['binary_sha256'] or c['prompt_sha256_end'] != c['prompt_sha256']:
    raise SystemExit('performance input changed during execution')
PY
  if [[ $rc -eq 0 ]]; then touch "$out/DONE" || rc=1; fi
  return "$rc"
}

serve
ANY_FAILED=0
for ((p=1;p<=PASSES;p++)); do
  for w in W1 W2; do
    for c in 1 8 32; do
      for pair in "bench-rel $BASE_BIN" "bench-src $CAND_BIN"; do
        label=${pair%% *}; bin=${pair#* }
        out="$OUT/$label/$p/$w/c$c"
        [[ -f "$out/DONE" ]] && { echo "skip $label/$p/$w/c$c"; continue; }
        echo "=== $label pass$p $w c$c $(date -u +%FT%TZ) ==="
        [[ $WARMUP -gt 0 ]] && run_one "$bin" "$w" "$c" "$p" "$label" "$out/warmup" "$WARMUP"
        if run_one "$bin" "$w" "$c" "$p" "$label" "$out" "$DURATION"; then
          echo "OK $label/$p/$w/c$c"
        else
          echo "FAIL $label/$p/$w/c$c"
          ANY_FAILED=1
        fi
      done
    done
  done
done
echo "RUNNER $TAG DONE $(date -u +%FT%TZ)"
exit "$ANY_FAILED"
