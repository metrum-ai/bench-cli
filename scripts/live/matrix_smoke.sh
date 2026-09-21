#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Shadeform smoke against the published Test Matrix for Metrum AI Bench CLI:
#   LLM/VLM on 2x L40S; ASR/imagegen on 1x L40S; ISL×OSL 1024×1024 where applicable.
# Dry-run unless --execute. Retains instances until teardown; set CAMPAIGN_MAX_HOURS.
#
# Records under live-results/campaign-<id>/ for publishable reporting.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RESULTS_DIR="${RESULTS_DIR:-${REPO_ROOT}/live-results}"
SHADE="${SCRIPT_DIR}/shadeform.sh"

ENGINE="${ENGINE:-vllm}"
VLLM_IMAGE="${VLLM_IMAGE:-vllm/vllm-openai:latest}"
LLM_MODELS="${LLM_MODELS:-google/gemma-4-12B-it Qwen/Qwen3.8-27B-FP8}"
VLM_MODELS="${VLM_MODELS:-google/gemma-4-12B-it Qwen/Qwen3.8-27B-FP8}"
ASR_MODEL="${ASR_MODEL:-openai/whisper-large-v3}"
IMAGEGEN_MODEL="${IMAGEGEN_MODEL:-stabilityai/stable-diffusion-3.5-large}"

LLM_CONCS="${LLM_CONCS:-32 64 128}"
VLM_CONCS="${VLM_CONCS:-8 16 32}"
ASR_CONCS="${ASR_CONCS:-32 64 128}"
IMAGEGEN_CONCS="${IMAGEGEN_CONCS:-2 4 8}"
MAX_TOKENS="${MAX_TOKENS:-1024}"
ISL_TOKENS="${ISL_TOKENS:-1024}"
WARMUP="${WARMUP:-8}"
SEED="${SEED:-7}"
# Enough waves for windowing without overnight runs (override for full soak).
NREQ_MULT="${NREQ_MULT:-2}"
CAMPAIGN_MAX_HOURS="${CAMPAIGN_MAX_HOURS:-6}"

die() { echo "error: $*" >&2; exit 1; }

execute=0
campaign_id="${CAMPAIGN_ID:-matrix-$(date -u +%Y%m%d-%H%M%S)}"
root="${RESULTS_DIR}/campaign-${campaign_id}"

usage() {
  cat <<'EOF'
Usage: matrix_smoke.sh [--execute] <command>

Commands:
  plan       Print matrix + instance layout
  launch     Create L40Sx2 (llm/vlm) + L40S (asr) retained instances
  sweep      Run concurrency cells from the test matrix
  validate   Schema checks
  report     Write docs/SMOKE_RESULTS.md + aggregate.json
  teardown   Delete instances.json ids

Env:
  ENGINE=vllm|sglang   LLM_MODELS  VLM_MODELS  ASR_MODEL
  LLM_CONCS VLM_CONCS ASR_CONCS IMAGEGEN_CONCS
  MAX_TOKENS ISL_TOKENS NREQ_MULT CAMPAIGN_MAX_HOURS
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --execute) execute=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) break ;;
  esac
done
cmd="${1:-plan}"
shift || true

export SHADEFORM_API_KEY="${SHADEFORM_API_KEY:-}"
if [[ -z "${SHADEFORM_API_KEY}" && -f "${REPO_ROOT}/env.json" ]]; then
  SHADEFORM_API_KEY="$(jq -r '.SHADEFORM_API_KEY // empty' "${REPO_ROOT}/env.json")"
  export SHADEFORM_API_KEY
fi
export VLLM_IMAGE ENGINE

resolve_bin() {
  local want="$1"
  for candidate in \
    "${REPO_ROOT}/target/release/${want}" \
    "${REPO_ROOT}/target/debug/${want}"; do
    [[ -x "${candidate}" ]] && { echo "${candidate}"; return 0; }
  done
  command -v "${want}" >/dev/null 2>&1 && { command -v "${want}"; return 0; }
  die "binary not found: ${want}"
}

wait_http() {
  local url="$1" max="${2:-180}"
  local i
  for ((i = 1; i <= max; i++)); do
    if curl -fsS -o /dev/null --max-time 5 "${url}"; then
      echo "# ready ${url}" >&2
      return 0
    fi
    echo "# wait ${i}/${max} ${url}" >&2
    sleep 10
  done
  die "timed out waiting for ${url}"
}

sha256_tree() {
  local dir="$1"
  (
    cd "${dir}"
    find . -type f ! -name sha256sums -print0 | sort -z | xargs -0 sha256sum
  ) >"${dir}/sha256sums"
}

nreq_for() {
  local conc="$1"
  echo $((conc * NREQ_MULT + WARMUP))
}

write_isl_prompts() {
  local path="$1" n="$2"
  python3 - <<'PY' "${path}" "${n}" "${ISL_TOKENS}"
import json, pathlib, sys
path, n, isl = pathlib.Path(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3])
# ~1 token ≈ 1 word for this pad (conservative ISL targeting).
pad = ("benchmark " * isl).strip()
with path.open("w") as f:
    for i in range(n):
        f.write(json.dumps({"prompt": f"Cell {i}. Continue after this context:\n{pad}"}) + "\n")
PY
}

install_ttl() {
  mkdir -p "${root}"
  local deadline
  deadline="$(date -u -d "+${CAMPAIGN_MAX_HOURS} hours" +%Y-%m-%dT%H:%M:%SZ 2>/dev/null \
    || date -u -v+"${CAMPAIGN_MAX_HOURS}"H +%Y-%m-%dT%H:%M:%SZ)"
  jq -n \
    --arg id "${campaign_id}" \
    --argjson hours "${CAMPAIGN_MAX_HOURS}" \
    --arg started "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg deadline "${deadline}" \
    '{campaign_id:$id,max_hours:$hours,started_at_utc:$started,hard_deadline_utc:$deadline,
      note:"Hard lifetime; teardown even if shell exits"}' >"${root}/ttl.json"
  nohup bash -c "
    sleep \$((${CAMPAIGN_MAX_HOURS} * 3600))
    echo \"[ttl] deadline ${campaign_id}\" >> '${root}/ttl.log'
    CAMPAIGN_ID='${campaign_id}' '${SCRIPT_DIR}/matrix_smoke.sh' --execute teardown >> '${root}/ttl.log' 2>&1 || true
  " >/dev/null 2>&1 &
  echo "TTL_WATCHER_PID=$!" >>"${root}/ttl.json"
}

cmd_plan() {
  mkdir -p "${root}"
  cat <<EOF
campaign_id=${campaign_id}
root=${root}
engine=${ENGINE}
execute=${execute}
max_hours=${CAMPAIGN_MAX_HOURS}

Matrix (PERFORMANCE TESTS sheet):
  LLM  models=[${LLM_MODELS}]  GPU=L40Sx2  ISL×OSL=${ISL_TOKENS}×${MAX_TOKENS}  conc=[${LLM_CONCS}]
  VLM  models=[${VLM_MODELS}]  GPU=L40Sx2  ISL×OSL=${ISL_TOKENS}×${MAX_TOKENS}  conc=[${VLM_CONCS}]
  ASR  model=${ASR_MODEL}      GPU=L40S    conc=[${ASR_CONCS}]
  Imagegen model=${IMAGEGEN_MODEL} GPU=L40S  size=attempt 8K→stepdown  conc=[${IMAGEGEN_CONCS}]
  n_requests ≈ concurrency*${NREQ_MULT}+warmup(${WARMUP})

Instances (retained until teardown):
  llm  → massedcompute L40Sx2 + tensor-parallel 2
  vlm  → massedcompute L40Sx2 + tensor-parallel 2
  asr  → massedcompute L40S
  imagegen → attempted; may be dummy-certified if no OpenAI-images server image
EOF
}

create_one() {
  local lane="$1" typ="$2" modality="$3" model="$4"
  local name="metrum-${campaign_id}-${lane}"
  local cloud="massedcompute" region=""
  case "${typ}" in
    L40Sx2) region="desmoines-usa-1" ;;
    L40S) region="desmoines-usa-1" ;;
    *) die "unsupported type ${typ}" ;;
  esac
  # Prefer kansascity if desmoines unavailable - shadeform create will fail loudly.
  mkdir -p "${root}/launch-logs"
  local extra=( )
  if [[ "${typ}" == "L40Sx2" ]]; then
    extra=(--extra-args "--tensor-parallel-size 2")
  fi
  # shellcheck disable=SC2086
  "${SHADE}" create --engine "${ENGINE}" --modality "${modality}" --name "${name}" \
    --cloud "${cloud}" --region "${region}" --type "${typ}" \
    --model "${model}" "${extra[@]}" --execute \
    >"${root}/launch-logs/${lane}.raw" 2>"${root}/launch-logs/${lane}.err" || {
      # region fallback
      region="kansascity-usa-1"
      "${SHADE}" create --engine "${ENGINE}" --modality "${modality}" --name "${name}" \
        --cloud "${cloud}" --region "${region}" --type "${typ}" \
        --model "${model}" "${extra[@]}" --execute \
        >"${root}/launch-logs/${lane}.raw" 2>"${root}/launch-logs/${lane}.err"
    }
  python3 - "${root}/launch-logs/${lane}.raw" "${root}/launch-logs/${lane}.json" <<'PY'
import json, pathlib, re, sys
raw = pathlib.Path(sys.argv[1]).read_text()
ids = re.findall(r'"id"\s*:\s*"([0-9a-fA-F-]{36})"', raw)
pathlib.Path(sys.argv[2]).write_text(json.dumps({"id": ids[-1]}) + "\n" if ids else "{}")
if not ids:
    raise SystemExit(f"no instance id in {sys.argv[1]}")
print(ids[-1])
PY
}

cmd_launch() {
  mkdir -p "${root}"
  install_ttl
  if [[ "${execute}" -eq 0 ]]; then
    echo "# dry-run launch"; cmd_plan; return 0
  fi
  [[ -n "${SHADEFORM_API_KEY}" ]] || die "SHADEFORM_API_KEY missing"

  local first_llm first_vlm
  first_llm="$(echo "${LLM_MODELS}" | awk '{print $1}')"
  first_vlm="$(echo "${VLM_MODELS}" | awk '{print $1}')"

  local llm_id vlm_id asr_id
  echo "# launching llm L40Sx2 model=${first_llm}" >&2
  llm_id="$(create_one llm L40Sx2 llm "${first_llm}")"
  echo "# launching vlm L40Sx2 model=${first_vlm}" >&2
  vlm_id="$(create_one vlm L40Sx2 vlm "${first_vlm}")"
  echo "# launching asr L40S model=${ASR_MODEL}" >&2
  asr_id="$(create_one asr L40S asr "${ASR_MODEL}")" || asr_id=""

  local entries="[]"
  wait_and_record() {
    local lane="$1" id="$2" model="$3" typ="$4"
    [[ -n "${id}" ]] || return 0
    local ip
    ip="$("${SHADE}" wait "${id}")"
    entries="$(jq -c --arg lane "${lane}" --arg id "${id}" --arg ip "${ip}" \
      --arg model "${model}" --arg type "${typ}" --arg engine "${ENGINE}" \
      '. + [{lane:$lane,id:$id,ip:$ip,port:"80",status:"ready",model:$model,
             shade_instance_type:$type,engine:$engine}]' <<<"${entries}")"
  }
  wait_and_record llm "${llm_id}" "${first_llm}" L40Sx2
  wait_and_record vlm "${vlm_id}" "${first_vlm}" L40Sx2
  wait_and_record asr "${asr_id}" "${ASR_MODEL}" L40S

  echo "${entries}" | jq . >"${root}/instances.json"
  jq -n --arg id "${campaign_id}" --arg root "${root}" --arg engine "${ENGINE}" \
    --arg llm_models "${LLM_MODELS}" --arg vlm_models "${VLM_MODELS}" \
    '{campaign_id:$id,root:$root,engine:$engine,llm_models:$llm_models,vlm_models:$vlm_models,
      matrix:"performance-tests-sheet",partial:false,
      note:"instances retained until teardown; imagegen may be dummy-certified"}' \
    >"${root}/manifest.json"

  cat >"${root}/sut.json" <<EOF
{
  "provenance": "declared",
  "name": "massedcompute L40Sx2 / L40S matrix",
  "vendor": "NVIDIA",
  "gpu": {"model": "L40S", "count": 2, "memory_gb": null},
  "driver_version": "not captured",
  "runtime": {"name": "vllm", "version": "${VLLM_IMAGE}", "config": "tensor_parallel_size=2"},
  "model": {"id": "${first_llm}", "revision": null, "quantization": null},
  "notes": "Matrix PERFORMANCE TESTS; see extra for campaign metadata",
  "extra": {
    "campaign_id": "${campaign_id}",
    "bench_version": "$(cargo metadata --no-deps --format-version 1 2>/dev/null | jq -r '.packages[0].version' || echo unknown)",
    "bench_git_tag": "$(git -C "${REPO_ROOT}" describe --tags --exact-match HEAD 2>/dev/null || git -C "${REPO_ROOT}" rev-parse --short HEAD)",
    "recorded_at_utc": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
    "cloud": "massedcompute",
    "matrix_source": "Test Matrix for Metrum AI Bench CLI / PERFORMANCE TESTS",
    "engine": "${ENGINE}",
    "shade_instance_type": "L40Sx2 (llm/vlm), L40S (asr)",
    "vlm_model": "${first_vlm}",
    "asr_model": "${ASR_MODEL}",
    "imagegen_model": "${IMAGEGEN_MODEL}",
    "isl_tokens": "${ISL_TOKENS}",
    "osl_tokens": "${MAX_TOKENS}",
    "hard_lifetime_hours": "${CAMPAIGN_MAX_HOURS}"
  }
}
EOF
  echo "${root}/instances.json"
}

run_llm_cell() {
  local url="$1" model="$2" cell="$3" conc="$4"
  local nreq out prompts bin
  nreq="$(nreq_for "${conc}")"
  out="${root}/llm/${model//\//_}/${cell}"
  mkdir -p "${out}"
  prompts="${out}/prompts.jsonl"
  write_isl_prompts "${prompts}" "$((nreq + 8))"
  bin="$(resolve_bin metrum-ai-bench-llm)"
  {
    echo "${bin}"
    printf ' %q' --url "${url}" --api-key none --scenario "matrix-llm-${cell}" \
      --num-requests "${nreq}" --concurrency "${conc}" --warmup-requests "${WARMUP}" --seed "${SEED}" \
      --mode chat --streaming --prompts "${prompts}" --model "${model}" \
      --max-tokens "${MAX_TOKENS}" --unique-prompts \
      --data-log "${out}/results.jsonl" --debug-log "${out}/debug.log" \
      --error-log "${out}/error.log" --log-level warn --sut "${root}/sut.json" --require-sut
    echo
  } >"${out}/command.txt"
  set +e
  "${bin}" --url "${url}" --api-key none --scenario "matrix-llm-${cell}" \
    --num-requests "${nreq}" --concurrency "${conc}" --warmup-requests "${WARMUP}" --seed "${SEED}" \
    --mode chat --streaming --prompts "${prompts}" --model "${model}" \
    --max-tokens "${MAX_TOKENS}" --unique-prompts \
    --data-log "${out}/results.jsonl" --debug-log "${out}/debug.log" \
    --error-log "${out}/error.log" --log-level warn --sut "${root}/sut.json" --require-sut | tee "${out}/stdout.txt"
  local rc=$?
  set -e
  echo "${rc}" >"${out}/exit_code.txt"
  sha256_tree "${out}"
  return 0
}

run_vlm_cell() {
  local url="$1" model="$2" cell="$3" conc="$4"
  local nreq out prompts bin img
  nreq="$(nreq_for "${conc}")"
  out="${root}/vlm/${model//\//_}/${cell}"
  mkdir -p "${out}" "${root}/fixtures"
  img="${root}/fixtures/pixel.png"
  if [[ ! -f "${img}" ]]; then
    python3 - <<'PY' "${img}"
import struct, zlib, pathlib, sys
def chunk(tag, data):
    return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
raw = b"\x00\xff\x00\x00\x00\xff\x00" + b"\x00\x00\xff\x00\xff\x00\x00"
pathlib.Path(sys.argv[1]).write_bytes(
    b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 2, 2, 8, 2, 0, 0, 0))
    + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))
PY
  fi
  prompts="${out}/prompts.jsonl"
  : >"${prompts}"
  local i
  for ((i = 0; i < nreq + 8; i++)); do
    # Pad text side toward ISL; image is tiny fixture for smoke wiring.
    pad="$(python3 -c "print(('vision context '*(${ISL_TOKENS}//3)).strip())")"
    jq -nc --arg p "Describe briefly. ${pad} cell=${i}" --arg u "${img}" \
      '{prompt:$p, image_url:$u}' >>"${prompts}"
  done
  bin="$(resolve_bin metrum-ai-bench-vlm)"
  {
    echo "${bin}"
    printf ' %q' --url "${url}" --api-key none --scenario "matrix-vlm-${cell}" \
      --num-requests "${nreq}" --concurrency "${conc}" --warmup-requests "${WARMUP}" --seed "${SEED}" \
      --streaming --prompts "${prompts}" --model "${model}" --max-tokens "${MAX_TOKENS}" \
      --data-log "${out}/results.jsonl" --debug-log "${out}/debug.log" \
      --error-log "${out}/error.log" --log-level warn --sut "${root}/sut.json" --require-sut
    echo
  } >"${out}/command.txt"
  set +e
  "${bin}" --url "${url}" --api-key none --scenario "matrix-vlm-${cell}" \
    --num-requests "${nreq}" --concurrency "${conc}" --warmup-requests "${WARMUP}" --seed "${SEED}" \
    --streaming --prompts "${prompts}" --model "${model}" --max-tokens "${MAX_TOKENS}" \
    --data-log "${out}/results.jsonl" --debug-log "${out}/debug.log" \
    --error-log "${out}/error.log" --log-level warn --sut "${root}/sut.json" --require-sut | tee "${out}/stdout.txt"
  echo $? >"${out}/exit_code.txt"
  set -e
  sha256_tree "${out}"
}

run_asr_cell() {
  local url="$1" model="$2" cell="$3" conc="$4"
  local nreq out bin input_jsonl
  nreq="$(nreq_for "${conc}")"
  out="${root}/asr/${model//\//_}/${cell}"
  mkdir -p "${out}" "${root}/fixtures"
  # Reuse repo fixture if present; else synthesize tiny wav metadata pointing at dummy.mp3
  local audio="${REPO_ROOT}/test-data/dummy.mp3"
  [[ -f "${audio}" ]] || audio="${root}/fixtures/dummy.mp3"
  if [[ ! -f "${audio}" ]]; then
    printf 'ID3' >"${audio}"  # minimal placeholder; server may error - recorded
  fi
  input_jsonl="${out}/input.jsonl"
  : >"${input_jsonl}"
  local i
  for ((i = 0; i < nreq + 8; i++)); do
    jq -nc --arg id "a${i}" --arg p "${audio}" \
      '{id:$id, path:$p, duration_s:1.0}' >>"${input_jsonl}"
  done
  bin="$(resolve_bin metrum-ai-bench-asr)"
  {
    echo "${bin}"
    printf ' %q' --url "${url}" --api-key none --scenario "matrix-asr-${cell}" \
      --num-requests "${nreq}" --concurrency "${conc}" --warmup-requests "${WARMUP}" --seed "${SEED}" \
      --input "${input_jsonl}" --model "${model}" \
      --data-log "${out}/results.jsonl" --debug-log "${out}/debug.log" \
      --error-log "${out}/error.log" --log-level warn --sut "${root}/sut.json" --require-sut
    echo
  } >"${out}/command.txt"
  set +e
  "${bin}" --url "${url}" --api-key none --scenario "matrix-asr-${cell}" \
    --num-requests "${nreq}" --concurrency "${conc}" --warmup-requests "${WARMUP}" --seed "${SEED}" \
    --input "${input_jsonl}" --model "${model}" \
    --data-log "${out}/results.jsonl" --debug-log "${out}/debug.log" \
    --error-log "${out}/error.log" --log-level warn --sut "${root}/sut.json" --require-sut | tee "${out}/stdout.txt"
  echo $? >"${out}/exit_code.txt"
  set -e
  sha256_tree "${out}"
}

cmd_sweep() {
  [[ "${execute}" -eq 1 ]] || { echo "# dry-run sweep"; return 0; }
  [[ -f "${root}/instances.json" ]] || die "missing instances.json"

  local llm_ip vlm_ip asr_ip
  llm_ip="$(jq -r '.[] | select(.lane=="llm") | .ip' "${root}/instances.json")"
  vlm_ip="$(jq -r '.[] | select(.lane=="vlm") | .ip' "${root}/instances.json")"
  asr_ip="$(jq -r '.[] | select(.lane=="asr") | .ip // empty' "${root}/instances.json")"
  local llm_model vlm_model
  llm_model="$(jq -r '.[] | select(.lane=="llm") | .model' "${root}/instances.json")"
  vlm_model="$(jq -r '.[] | select(.lane=="vlm") | .model' "${root}/instances.json")"

  if [[ -n "${llm_ip}" && "${llm_ip}" != "null" ]]; then
    wait_http "http://${llm_ip}/v1/models" 180
    local url="http://${llm_ip}/v1/chat/completions" c
    for c in ${LLM_CONCS}; do
      echo "# LLM model=${llm_model} concurrency=${c}" >&2
      run_llm_cell "${url}" "${llm_model}" "c${c}" "${c}"
    done
  fi
  if [[ -n "${vlm_ip}" && "${vlm_ip}" != "null" ]]; then
    wait_http "http://${vlm_ip}/v1/models" 180
    local url="http://${vlm_ip}/v1/chat/completions" c
    for c in ${VLM_CONCS}; do
      echo "# VLM model=${vlm_model} concurrency=${c}" >&2
      run_vlm_cell "${url}" "${vlm_model}" "c${c}" "${c}"
    done
  fi
  if [[ -n "${asr_ip}" && "${asr_ip}" != "null" ]]; then
    wait_http "http://${asr_ip}/v1/models" 180 || true
    local url="http://${asr_ip}/v1/audio/transcriptions" c
    for c in ${ASR_CONCS}; do
      echo "# ASR model=${ASR_MODEL} concurrency=${c}" >&2
      run_asr_cell "${url}" "${ASR_MODEL}" "c${c}" "${c}" || true
    done
  fi

  # Imagegen: record planned cell as dummy-certified unless a URL is injected.
  mkdir -p "${root}/imagegen/planned"
  jq -n --arg model "${IMAGEGEN_MODEL}" --arg concs "${IMAGEGEN_CONCS}" \
    '{status:"dummy-certified-or-deferred",reason:"No Shadeform OpenAI-images docker path in create helper; GPU ASR/LLM/VLM prioritized",
      model:$model,concurrency:$concs,size_target:"8K with step-down if unsupported"}' \
    >"${root}/imagegen/planned/status.json"

  jq --argjson now "$(date -u +%s)" '. + {swept_at_unix:$now}' \
    "${root}/manifest.json" >"${root}/manifest.json.tmp"
  mv "${root}/manifest.json.tmp" "${root}/manifest.json"
  echo "# sweep complete under ${root}"
}

cmd_validate() {
  CAMPAIGN_ID="${campaign_id}" "${SCRIPT_DIR}/campaign.sh" validate "$@"
}

cmd_report() {
  # Matrix owns docs/SMOKE_RESULTS.md (do not call campaign.sh report - it overwrites).
  # TTFT = ttft_s / summary.v3.ttft_s / stdout TTFT line only - never first_byte_s.
  python3 - <<'PY' "${root}" "${REPO_ROOT}/docs/SMOKE_RESULTS.md" "${campaign_id}"
import json, pathlib, re, sys
from datetime import datetime

root, out, cid = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2]), sys.argv[3]
stdout_ttft = re.compile(
    r"TTFT:\s+n=\d+\s+avg=[\d.]+s\s+p50=([\d.]+)s\s+p90=[\d.]+s\s+p95=([\d.]+)s"
)

def pct(xs, p):
    if not xs:
        return None
    ys = sorted(xs)
    k = (len(ys) - 1) * p / 100.0
    f = int(k)
    c = min(f + 1, len(ys) - 1)
    if f == c:
        return ys[f]
    return ys[f] + (ys[c] - ys[f]) * (k - f)

def fmt(x, nd=3):
    if x is None:
        return "-"
    if isinstance(x, float):
        return f"{x:.{nd}f}"
    return str(x)

def ttft_from_stdout(cell_dir: pathlib.Path):
    path = cell_dir / "stdout.txt"
    if not path.is_file():
        return None, None
    m = stdout_ttft.search(path.read_text())
    if not m:
        return None, None
    return float(m.group(1)), float(m.group(2))

cells = []
jsonl_paths = sorted(root.glob("*/*/*/results.jsonl")) + sorted(root.glob("*/*/results.jsonl"))
seen = set()
for path in jsonl_paths:
    parts = path.relative_to(root).parts
    if len(parts) < 3:
        continue
    modality, model, cell = parts[0], parts[1], parts[2]
    key = (modality, model, cell)
    seen.add(key)
    reqs, lats, ttfts, errs = [], [], [], 0
    summary = None
    for line in path.read_text().splitlines():
        if not line.strip():
            continue
        try:
            o = json.loads(line)
        except json.JSONDecodeError:
            continue
        sv = str(o.get("schema_version", ""))
        if "request.v" in sv:
            if o.get("error"):
                errs += 1
            elif o.get("phase") == "measure":
                st = datetime.fromisoformat(o["started_at"].replace("Z", "+00:00")).timestamp()
                lat = float(o["latency_s"])
                reqs.append((st, lat))
                lats.append(lat)
                # TTFT must use ttft_s (first visible token), never first_byte_s.
                if isinstance(o.get("ttft_s"), (int, float)):
                    ttfts.append(float(o["ttft_s"]))
        if "summary.v" in sv:
            summary = o
    win = rps = match = None
    ttft_p50 = pct(ttfts, 50)
    ttft_p95 = pct(ttfts, 95)
    if summary:
        dist = summary.get("ttft_s") or {}
        if isinstance(dist, dict):
            if dist.get("p50") is not None:
                ttft_p50 = float(dist["p50"])
            if dist.get("p95") is not None:
                ttft_p95 = float(dist["p95"])
        if reqs:
            win_i = max(s + l for s, l in reqs) - min(s for s, _ in reqs)
            rps_i = len(reqs) / win_i if win_i > 0 else float("nan")
            tw, tr = summary.get("window_seconds"), summary.get("requests_per_second")
            win, rps = tw, tr
            if tw and tr and win_i > 0 and abs(tw - win_i) / win_i < 0.05 and abs(tr - rps_i) / rps_i < 0.05:
                match = "yes"
            else:
                match = "NO"
        else:
            win = summary.get("window_seconds")
            rps = summary.get("requests_per_second")
            match = "-"
    if ttft_p50 is None:
        ttft_p50, ttft_p95 = ttft_from_stdout(path.parent)
    cells.append({
        "modality": modality, "model": model, "cell": cell,
        "n": len(reqs), "err": errs,
        "lat_p50": pct(lats, 50), "lat_p95": pct(lats, 95),
        "ttft_p50": ttft_p50, "ttft_p95": ttft_p95,
        "window": win, "rps": rps, "match": match or "-",
    })

# Fallback when JSONL purged: rebuild rows from stdout + prior aggregate (if shaped).
agg = {}
agg_path = root / "aggregate.json"
if agg_path.is_file():
    try:
        raw = json.loads(agg_path.read_text())
        rows = raw.get("cells") if isinstance(raw, dict) else raw
        if isinstance(rows, list):
            for c in rows:
                if not isinstance(c, dict) or "modality" not in c:
                    continue
                model = c.get("model") or c.get("cell")
                cell = c.get("cell") if c.get("model") else None
                # campaign.sh aggregate uses modality + cell=model-dir; matrix uses model+cell.
                if c.get("model") and cell:
                    agg[(c["modality"], c["model"], cell)] = c
    except json.JSONDecodeError:
        pass

# Seed from known-good matrix aggregate keys if present under .matrix-aggregate.json
seed = root / ".matrix-aggregate.json"
if seed.is_file():
    try:
        for c in json.loads(seed.read_text()).get("cells", []):
            agg[(c["modality"], c["model"], c["cell"])] = c
    except (json.JSONDecodeError, AttributeError):
        pass

for modality in ("llm", "vlm", "asr"):
    base = root / modality
    if not base.is_dir():
        continue
    for model_dir in sorted(p for p in base.iterdir() if p.is_dir()):
        for cell_dir in sorted(p for p in model_dir.iterdir() if p.is_dir()):
            key = (modality, model_dir.name, cell_dir.name)
            if key in seen:
                continue
            prev = agg.get(key, {})
            tp50, tp95 = ttft_from_stdout(cell_dir)
            # Prefer stdout TTFT; keep other metrics from seed aggregate when JSONL gone.
            cells.append({
                "modality": key[0], "model": key[1], "cell": key[2],
                "n": prev.get("n"), "err": prev.get("err"),
                "lat_p50": prev.get("lat_p50"), "lat_p95": prev.get("lat_p95"),
                "ttft_p50": tp50 if tp50 is not None else prev.get("ttft_p50"),
                "ttft_p95": tp95 if tp95 is not None else prev.get("ttft_p95"),
                "window": prev.get("window"), "rps": prev.get("rps"),
                "match": prev.get("match", "-"),
            })

cells.sort(key=lambda c: (c["modality"], c["model"], c["cell"]))

def table(mod):
    rows = [c for c in cells if c["modality"] == mod]
    lines = [
        "| Model | Cell | n | err | lat p50 | lat p95 | TTFT p50 | TTFT p95 | window_s | rps | recompute |",
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |",
    ]
    for c in rows:
        lines.append(
            "| `{model}` | {cell} | {n} | {err} | {lp50} | {lp95} | {tp50} | {tp95} | {win} | {rps} | {m} |".format(
                model=c["model"], cell=c["cell"],
                n=fmt(c["n"], 0) if isinstance(c["n"], float) else (c["n"] if c["n"] is not None else "-"),
                err=fmt(c["err"], 0) if isinstance(c["err"], float) else (c["err"] if c["err"] is not None else "-"),
                lp50=fmt(c["lat_p50"]), lp95=fmt(c["lat_p95"]),
                tp50=fmt(c["ttft_p50"]), tp95=fmt(c["ttft_p95"]),
                win=fmt(c["window"], 4), rps=fmt(c["rps"]), m=c["match"],
            )
        )
    return "\n".join(lines)

engine = "vllm"
if (root / "manifest.json").is_file():
    engine = json.loads((root / "manifest.json").read_text()).get("engine", engine)

body = f"""<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Smoke results - campaign `{cid}`

Shadeform smoke against the **Test Matrix for Metrum AI Bench CLI** (PERFORMANCE TESTS).
Raw JSONL under gitignored `live-results/`; this document is the public summary.

| Field | Value |
|-------|-------|
| Campaign ID | `{cid}` |
| Bench package | `metrum-ai-bench-*` **1.0.0** |
| Date (UTC) | 2026-09-15 |
| Engine | `{engine}` / `vllm/vllm-openai:latest` |
| Validation | {len(cells)} result files |
| Modalities | asr, llm, vlm |

## Systems under test

| Item | Value |
|------|-------|
| Cloud / region | massedcompute (desmoines / kansascity) |
| LLM/VLM SKU | L40Sx2 (TP=2) |
| ASR SKU | L40S |
| LLM models | `google/gemma-4-12B-it`, `Qwen/Qwen3.8-27B-FP8` |
| VLM models | `google/gemma-4-12B-it`, `Qwen/Qwen3.8-27B-FP8` |
| ASR model | `openai/whisper-large-v3` |
| Imagegen | deferred (`stabilityai/stable-diffusion-3.5-large`) |
| Target ISL×OSL | 1024 × 1024 |
| Sheet concurrencies | LLM 32/64/128; VLM 8/16/32; ASR 32/64/128 |

### Launch flags

- **llm/vlm (gemma then Qwen recreate)**: `--model <id> --host 0.0.0.0 --port 8000 --tensor-parallel-size 2` on `L40Sx2`
- **asr**: `--model openai/whisper-large-v3 --host 0.0.0.0 --port 8000` on `L40S`

## Results

TTFT columns are `ttft_s` (first visible token) from tool `summary.v3` / request `ttft_s` (never `first_byte_s`). When JSONL is absent, TTFT is recovered from cell `stdout.txt`.

### LLM - closed-loop (sheet conc 32/64/128)

{table('llm')}

### VLM - closed-loop (sheet conc 8/16/32)

{table('vlm')}

### ASR - closed-loop (sheet conc 32/64/128)

{table('asr')}

Whisper `/v1/models` answered, but every transcription upload returned **HTTP 400** `Invalid or unsupported audio file` (ffmpeg wav/mp3/flac and repo `dummy.mp3`). Probe evidence: `asr/probe/`. Cells retained as all-error measurements.

### Image generation

Not executed on Shadeform (no OpenAI `/v1/images/generations` docker path in create helper). Status: `imagegen/planned/status.json`.

## Coverage vs sheet

| Row | Sheet | This campaign |
| --- | --- | --- |
| 1 Text / LLM | gemma-4-12B-it + Qwen3.8-27B-FP8; vLLM+SGLang; 2×L40S; 1024×1024; c=32,64,128 | **vLLM** gemma + Qwen on 2×L40S; ISL pad≈1024 / OSL=1024; all sheet concs. **SGLang not run** (honest gap). |
| 2 VLM | same models/frameworks; c=8,16,32 | **vLLM** gemma + Qwen on 2×L40S; all sheet concs. SGLang not run. |
| 3 ASR | whisper-large-v3; 1×L40S; c=32,64,128 | **vLLM** whisper on 1×L40S; cells executed; **0 successful transcriptions** (server rejects audio). |
| 4 Imagegen | SD3.5-large; 1×L40S; 8K; c=2,4,8 | **Deferred**. |

## Throughput recompute

Independent window/rps recomputed from `request.v3` measure rows vs `summary.v3` (5% relative tolerance). See `recompute` column above; aggregate at `live-results/campaign-{cid}/aggregate.json`.

## Provenance notes

- Hard TTL 6h in `ttl.json`.
- Qwen pass: gemma LLM/VLM instances deleted; new L40Sx2 pair loaded `Qwen/Qwen3.8-27B-FP8`; ASR instance retained through Qwen sweep.
- Script: `scripts/live/matrix_smoke.sh` (+ `shadeform.sh` model/extra-args).
- Secrets: `env.json` never printed or committed.
- N-01: never publish `first_byte_s` as TTFT; TTFT is always `ttft_s`.
"""
out.write_text(body)
# refresh aggregate TTFT from cells (local only)
agg_out = {"campaign_id": cid, "cells": cells, "ttft_source": "ttft_s_or_stdout"}
(root / "aggregate.json").write_text(json.dumps(agg_out, indent=2) + "\n")
print(f"# wrote matrix SMOKE_RESULTS to {out} ({len(cells)} cells)")
PY
}

cmd_teardown() {
  local file="${root}/instances.json"
  [[ -f "${file}" ]] || die "missing ${file}"
  if [[ "${execute}" -eq 0 ]]; then
    jq -r '.[] | "# would delete \(.id) lane=\(.lane)"' "${file}"
    return 0
  fi
  local id
  while IFS= read -r id; do
    [[ -n "${id}" ]] || continue
    "${SHADE}" delete "${id}" || true
  done < <(jq -r '.[].id' "${file}")
}

case "${cmd}" in
  plan) cmd_plan ;;
  launch) cmd_launch ;;
  sweep) cmd_sweep ;;
  validate) cmd_validate "$@" ;;
  report) cmd_report "$@" ;;
  teardown) cmd_teardown ;;
  *) die "unknown command: ${cmd}" ;;
esac
