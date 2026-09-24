#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# On-SUT bake-off: NVIDIA AIPerf vs the already-served vLLM endpoint.
# Uses the same Hub prompt-library mix as metrum-ai-bench-cli-strategic.
set -euo pipefail

RESULTS="${RESULTS_REMOTE:-/tmp/metrum-e2e}"
MODEL_NAME="${SERVED_NAME:-sut}"
# HF tokenizer for client-side counting (served name "sut" is not a Hub id).
TOKENIZER="${AIPERF_TOKENIZER:-Qwen/Qwen3.8-27B}"
URL="${AIPERF_URL:-http://127.0.0.1:8000}"
OUT="${RESULTS}/aiperf"
MIX="${RESULTS}/prompts/mix.jsonl"
SWEEP="${AIPERF_SWEEP:-1,2,4,8,16,32,64}"
REQS_PER_STAGE="${AIPERF_REQUESTS_PER_STAGE:-16}"
MAX_TOKENS="${AIPERF_MAX_TOKENS:-256}"
TIMING="${OUT}/timings.json"

log() { echo "[aiperf-bakeoff $(date -u +%Y-%m-%dT%H:%M:%SZ)] $*"; }

mkdir -p "${OUT}"
: >"${OUT}/bakeoff.log"
# Fresh stage outputs for this attempt
rm -f "${OUT}/stages.jsonl"
rm -rf "${OUT}"/c[0-9]*

ts() { date -u +%s; }

SETUP_START="$(ts)"
log "installing AIPerf into ${RESULTS}/aiperf-venv"
if [[ ! -x "${RESULTS}/aiperf-venv/bin/aiperf" ]]; then
  if ! python3 -m venv "${RESULTS}/aiperf-venv" 2>/tmp/aiperf-venv.err; then
    log "venv failed; installing python3-venv"
    sudo DEBIAN_FRONTEND=noninteractive apt-get update -qq || true
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq python3-venv python3-pip || true
    rm -rf "${RESULTS}/aiperf-venv"
    python3 -m venv "${RESULTS}/aiperf-venv"
  fi
  # shellcheck disable=SC1091
  source "${RESULTS}/aiperf-venv/bin/activate"
  pip install -q --upgrade pip
  pip install -q "aiperf"
else
  # shellcheck disable=SC1091
  source "${RESULTS}/aiperf-venv/bin/activate"
fi
AIPERF_VER="$(python -c 'import importlib.metadata as m; print(m.version("aiperf"))' 2>/dev/null || aiperf --version 2>/dev/null | head -1 || echo unknown)"
SETUP_END="$(ts)"
log "AIPerf ready version=${AIPERF_VER} tokenizer=${TOKENIZER} setup_s=$((SETUP_END - SETUP_START))"

[[ -s "${MIX}" ]] || { echo "missing prompt mix ${MIX}" >&2; exit 1; }

python3 - <<PY
import json
from pathlib import Path
src = Path("${MIX}")
dst = Path("${OUT}/aiperf-input.jsonl")
n = 0
with src.open(encoding="utf-8") as fin, dst.open("w", encoding="utf-8") as fout:
    for line in fin:
        row = json.loads(line)
        text = row.get("prompt") or row.get("text")
        if not text:
            continue
        out = {"text": text}
        # Prefer fixed completion length when the mix carries a target.
        osl = row.get("target_output_tokens") or row.get("output_tokens")
        if osl:
            try:
                out["output_length"] = int(osl)
            except (TypeError, ValueError):
                pass
        fout.write(json.dumps(out, ensure_ascii=False) + "\n")
        n += 1
print(f"wrote {dst} rows={n}")
if n < 8:
    raise SystemExit("aiperf input too small")
PY

# Sanity: endpoint up
curl -fsS --max-time 5 "${URL}/v1/models" >/dev/null

RUN_START="$(ts)"
declare -a STAGE_TIMES=()
IFS=',' read -r -a CONCS <<<"${SWEEP}"
for c in "${CONCS[@]}"; do
  stage_dir="${OUT}/c${c}"
  mkdir -p "${stage_dir}"
  s="$(ts)"
  log "aiperf profile concurrency=${c} requests=${REQS_PER_STAGE}"
  # Headless; reuse Hub prompts; match strategic streaming + bounded OSL.
  # --tokenizer must be a real HF id (served name "sut" is not).
  # --use-server-token-count is a fallback if Hub tokenizer download fails.
  set +e
  aiperf profile \
    --model "${MODEL_NAME}" \
    --tokenizer "${TOKENIZER}" \
    --url "${URL}" \
    --endpoint-type chat \
    --endpoint /v1/chat/completions \
    --streaming \
    --concurrency "${c}" \
    --request-count "${REQS_PER_STAGE}" \
    --input-file "${OUT}/aiperf-input.jsonl" \
    --custom-dataset-type single_turn \
    --output-tokens-mean "${MAX_TOKENS}" \
    --output-tokens-stddev 0 \
    --artifact-dir "${stage_dir}" \
    --ui none \
    >"${stage_dir}/stdout.txt" 2>"${stage_dir}/stderr.txt"
  rc=$?
  if [[ "${rc}" -ne 0 ]]; then
    # Retry once with server-side token counts (no Hub tokenizer needed).
    log "concurrency=${c} failed rc=${rc}; retry with --use-server-token-count"
    tail -n 30 "${stage_dir}/stderr.txt" >&2 || true
    aiperf profile \
      --model "${MODEL_NAME}" \
      --use-server-token-count \
      --url "${URL}" \
      --endpoint-type chat \
      --endpoint /v1/chat/completions \
      --streaming \
      --concurrency "${c}" \
      --request-count "${REQS_PER_STAGE}" \
      --input-file "${OUT}/aiperf-input.jsonl" \
      --custom-dataset-type single_turn \
      --output-tokens-mean "${MAX_TOKENS}" \
      --output-tokens-stddev 0 \
      --artifact-dir "${stage_dir}" \
      --ui none \
      >"${stage_dir}/stdout.txt" 2>"${stage_dir}/stderr.txt"
    rc=$?
  fi
  set -e
  e="$(ts)"
  echo "{\"concurrency\":${c},\"seconds\":$((e - s)),\"rc\":${rc}}" >>"${OUT}/stages.jsonl"
  log "concurrency=${c} done rc=${rc} seconds=$((e - s))"
  STAGE_TIMES+=("${c}:$((e - s))")
done
RUN_END="$(ts)"

python3 - <<PY
import json, time
from pathlib import Path
out = Path("${OUT}")
timings = {
  "tool": "aiperf",
  "version": "${AIPERF_VER}",
  "setup_seconds": ${SETUP_END} - ${SETUP_START},
  "run_seconds": ${RUN_END} - ${RUN_START},
  "total_seconds": ${RUN_END} - ${SETUP_START},
  "sweep": "${SWEEP}",
  "requests_per_stage": ${REQS_PER_STAGE},
  "max_tokens": ${MAX_TOKENS},
  "model": "${MODEL_NAME}",
  "url": "${URL}",
  "dataset": {
    "source_mix": "${MIX}",
    "aiperf_input": str(out / "aiperf-input.jsonl"),
    "hub": "https://huggingface.co/datasets/metrum-ai/prompt-library",
  },
  "stages": [],
  "finished_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
}
stages_path = out / "stages.jsonl"
if stages_path.exists():
  for line in stages_path.read_text(encoding="utf-8").splitlines():
    if line.strip():
      timings["stages"].append(json.loads(line))
(out / "timings.json").write_text(json.dumps(timings, indent=2) + "\n", encoding="utf-8")
print(json.dumps(timings, indent=2))
PY

log "aiperf bake-off complete under ${OUT}"
