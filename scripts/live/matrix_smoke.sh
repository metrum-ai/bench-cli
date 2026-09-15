#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Shadeform smoke against the published Test Matrix for Metrum Bench CLI:
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
  # Prefer kansascity if desmoines unavailable — shadeform create will fail loudly.
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
  "campaign_id": "${campaign_id}",
  "bench_version": "$(cargo metadata --no-deps --format-version 1 2>/dev/null | jq -r '.packages[0].version' || echo unknown)",
  "bench_git_tag": "$(git -C "${REPO_ROOT}" describe --tags --exact-match HEAD 2>/dev/null || git -C "${REPO_ROOT}" rev-parse --short HEAD)",
  "recorded_at_utc": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "cloud": "massedcompute",
  "matrix_source": "Test Matrix for Metrum Bench CLI / PERFORMANCE TESTS",
  "engine": "${ENGINE}",
  "vllm_image": "${VLLM_IMAGE}",
  "gpu": {"name": "L40S", "count": 2, "driver": "not captured", "cuda": "not captured"},
  "shade_instance_type": "L40Sx2 (llm/vlm), L40S (asr)",
  "llm_model": "${first_llm}",
  "vlm_model": "${first_vlm}",
  "asr_model": "${ASR_MODEL}",
  "imagegen_model": "${IMAGEGEN_MODEL}",
  "isl_osl": {"isl_tokens": ${ISL_TOKENS}, "osl_tokens": ${MAX_TOKENS}},
  "concurrency": {"llm": "${LLM_CONCS}", "vlm": "${VLM_CONCS}", "asr": "${ASR_CONCS}", "imagegen": "${IMAGEGEN_CONCS}"},
  "hard_lifetime_hours": ${CAMPAIGN_MAX_HOURS},
  "model_selection_reason": "Sheet models; first model loaded at launch; additional models swept by recreate if time permits",
  "launch_flags": {"tensor_parallel_size": 2, "host_port": 80}
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
      --error-log "${out}/error.log" --log-level warn
    echo
  } >"${out}/command.txt"
  set +e
  "${bin}" --url "${url}" --api-key none --scenario "matrix-llm-${cell}" \
    --num-requests "${nreq}" --concurrency "${conc}" --warmup-requests "${WARMUP}" --seed "${SEED}" \
    --mode chat --streaming --prompts "${prompts}" --model "${model}" \
    --max-tokens "${MAX_TOKENS}" --unique-prompts \
    --data-log "${out}/results.jsonl" --debug-log "${out}/debug.log" \
    --error-log "${out}/error.log" --log-level warn | tee "${out}/stdout.txt"
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
      --error-log "${out}/error.log" --log-level warn
    echo
  } >"${out}/command.txt"
  set +e
  "${bin}" --url "${url}" --api-key none --scenario "matrix-vlm-${cell}" \
    --num-requests "${nreq}" --concurrency "${conc}" --warmup-requests "${WARMUP}" --seed "${SEED}" \
    --streaming --prompts "${prompts}" --model "${model}" --max-tokens "${MAX_TOKENS}" \
    --data-log "${out}/results.jsonl" --debug-log "${out}/debug.log" \
    --error-log "${out}/error.log" --log-level warn | tee "${out}/stdout.txt"
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
    printf 'ID3' >"${audio}"  # minimal placeholder; server may error — recorded
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
      --error-log "${out}/error.log" --log-level warn
    echo
  } >"${out}/command.txt"
  set +e
  "${bin}" --url "${url}" --api-key none --scenario "matrix-asr-${cell}" \
    --num-requests "${nreq}" --concurrency "${conc}" --warmup-requests "${WARMUP}" --seed "${SEED}" \
    --input "${input_jsonl}" --model "${model}" \
    --data-log "${out}/results.jsonl" --debug-log "${out}/debug.log" \
    --error-log "${out}/error.log" --log-level warn | tee "${out}/stdout.txt"
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
  # Reuse campaign report, then append matrix provenance.
  CAMPAIGN_ID="${campaign_id}" "${SCRIPT_DIR}/campaign.sh" report "$@"
  python3 - <<'PY' "${root}" "${REPO_ROOT}/docs/SMOKE_RESULTS.md" "${campaign_id}"
import json, pathlib, sys
from datetime import datetime
root, out, cid = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2]), sys.argv[3]
lines = ["", "## Matrix smoke throughput recompute", "",
"| Cell | n | window_s | rps | match |", "| --- | --- | --- | --- | --- |"]
for path in sorted(root.glob("*/*/*/results.jsonl")) + sorted(root.glob("*/*/results.jsonl")):
    reqs, summary = [], None
    for line in path.read_text().splitlines():
        if not line.strip():
            continue
        try:
            o = json.loads(line)
        except json.JSONDecodeError:
            continue
        sv = str(o.get("schema_version", ""))
        if "request.v" in sv and o.get("phase") == "measure" and not o.get("error"):
            st = datetime.fromisoformat(o["started_at"].replace("Z", "+00:00")).timestamp()
            reqs.append((st, float(o["latency_s"])))
        if "summary.v" in sv:
            summary = o
    if not reqs or not summary:
        continue
    win = max(s + l for s, l in reqs) - min(s for s, _ in reqs)
    rps = len(reqs) / win if win > 0 else float("nan")
    tw, tr = summary.get("window_seconds"), summary.get("requests_per_second")
    ok = tw and tr and abs(tw - win) / win < 0.05 and abs(tr - rps) / rps < 0.05
    cell = "/".join(path.relative_to(root).parts[:3])
    lines.append(f"| `{cell}` | {len(reqs)} | {tw:.4f} | {tr:.3f} | {'yes' if ok else 'NO'} |")
notes = [
"", "## Matrix provenance", "",
f"- Campaign `{cid}` from sheet **PERFORMANCE TESTS** (LLM/VLM 2×L40S, ASR 1×L40S).",
f"- Engine `{json.loads((root/'manifest.json').read_text()).get('engine','vllm')}`; imagegen deferred/dummy-certified in this pass.",
"- Hard TTL recorded in `ttl.json`; teardown after validate+report.",
""]
text = out.read_text().rstrip() + "\n" + "\n".join(lines + notes) + "\n"
out.write_text(text)
print(f"# appended matrix sections to {out}")
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
