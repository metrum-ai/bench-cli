#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# End-of-work GPU campaign: launch retained parallel Shadeform lanes,
# run sweeps, validate, backup. Dry-run unless --execute.
# Does not delete instances; use "teardown" after backup+case study.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RESULTS_DIR="${RESULTS_DIR:-${REPO_ROOT}/live-results}"
SHADE="${SCRIPT_DIR}/shadeform.sh"
LANES="${CAMPAIGN_LANES:-llm vlm}"

die() { echo "error: $*" >&2; exit 1; }

campaign_id="${CAMPAIGN_ID:-$(date -u +%Y%m%d-%H%M%S)}"
root="${RESULTS_DIR}/campaign-${campaign_id}"
execute=0

usage() {
  cat <<'EOF'
Usage: campaign.sh [--execute] <command>

Commands:
  plan       Print the sweep matrix and instance layout (no API)
  launch     Create one retained instance per lane (parallel POSTs)
  sweep      Run the retained matrix against instances.json
  validate   Schema / completeness checks on live-results
  backup     restic snapshot of the campaign directory
  teardown   Delete every id in instances.json
  demo       Print public demo commands (no spend)

Default is dry-run. --execute is required to create, sweep, backup, or delete.
Keep instances until backup restore-check and the case study are done.
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

cmd_plan() {
  mkdir -p "${root}"
  cat <<EOF
campaign_id=${campaign_id}
root=${root}
lanes=${LANES}
execute=${execute}

LLM cells:
  concurrency 1 2 4 8   closed-loop  warmup=8 n=64 max_tokens=128 seed=7
  request-rate 4 8 16   arrival=constant max-concurrency=8

VLM cells:
  concurrency 1 2 4     streaming     warmup=8 n=32 max_tokens=64 seed=7

ASR / imagegen:
  launched only if CAMPAIGN_LANES includes asr or imagegen and an image is
  practical; otherwise dummy-certified and labeled in manifest.json.

Instances stay up until: campaign.sh teardown --execute
EOF
}

cmd_launch() {
  mkdir -p "${root}"
  if [[ "${execute}" -eq 0 ]]; then
    echo "# dry-run launch lanes: ${LANES}"
    for lane in ${LANES}; do
      echo "# would: ${SHADE} create --engine vllm --modality ${lane} --name metrum-${campaign_id}-${lane} --execute"
    done
    echo "# write ${root}/instances.json after real create"
    return 0
  fi
  local pids=() names=()
  mkdir -p "${root}/launch-logs"
  for lane in ${LANES}; do
    case "${lane}" in
      llm|vlm) ;;
      *)
        echo "# skip create for ${lane}: shadeform.sh create supports llm|vlm only" >&2
        continue
        ;;
    esac
    local name="metrum-${campaign_id}-${lane}"
    names+=("${lane}")
    (
      "${SHADE}" create --engine vllm --modality "${lane}" --name "${name}" --execute \
        >"${root}/launch-logs/${lane}.json"
    ) &
    pids+=("$!")
  done
  local status=0
  local pid
  for pid in "${pids[@]+"${pids[@]}"}"; do
    wait "${pid}" || status=1
  done
  [[ "${status}" -eq 0 ]] || die "one or more parallel creates failed; inspect ${root}/launch-logs"
  local entries="[]"
  local lane id
  for lane in "${names[@]+"${names[@]}"}"; do
    id="$(jq -r '.id // .instance_id // empty' "${root}/launch-logs/${lane}.json")"
    [[ -n "${id}" ]] || die "no instance id in launch-logs/${lane}.json"
    entries="$(jq --arg lane "${lane}" --arg id "${id}" \
      '. + [{lane:$lane,id:$id,status:"created"}]' <<<"${entries}")"
  done
  echo "${entries}" | jq . >"${root}/instances.json"
  echo "# launched; waiting for IPs"
  local row ip
  local waited="[]"
  while read -r row; do
    lane="$(jq -r .lane <<<"${row}")"
    id="$(jq -r .id <<<"${row}")"
    ip="$("${SHADE}" wait "${id}")"
    waited="$(jq --arg lane "${lane}" --arg id "${id}" --arg ip "${ip}" \
      '. + [{lane:$lane,id:$id,ip:$ip,status:"ready"}]' <<<"${waited}")"
  done < <(jq -c '.[]' "${root}/instances.json")
  echo "${waited}" | jq . >"${root}/instances.json"
  jq -n --arg id "${campaign_id}" --arg root "${root}" \
    '{campaign_id:$id,root:$root,partial:false,note:"instances retained until teardown"}' \
    >"${root}/manifest.json"
  echo "${root}/instances.json"
}

cmd_teardown() {
  local file="${root}/instances.json"
  [[ -f "${file}" ]] || die "missing ${file}"
  if [[ "${execute}" -eq 0 ]]; then
    echo "# dry-run teardown:"
    jq -r '.[] | "would delete \(.id) lane=\(.lane)"' "${file}"
    return 0
  fi
  local id
  while read -r id; do
    [[ -n "${id}" ]] || continue
    "${SHADE}" delete "${id}" || true
  done < <(jq -r '.[].id' "${file}")
}

cmd_demo() {
  cat <<'EOF'
# After a campaign directory exists (gitignored live-results/):

metrum-ai-bench-llm \
  --url http://HOST/v1/chat/completions --api-key dummy \
  --scenario demo-llm --num-requests 64 --concurrency 4 \
  --warmup-requests 8 --seed 7 --streaming --mode chat \
  --prompts prompts.jsonl --model Qwen/Qwen2.5-7B-Instruct \
  --max-tokens 128 --data-log demo-llm.jsonl

metrum-ai-bench-vlm \
  --url http://HOST/v1/chat/completions --api-key dummy \
  --scenario demo-vlm --num-requests 32 --concurrency 2 \
  --warmup-requests 8 --seed 7 --streaming \
  --prompts vlm.jsonl --model Qwen/Qwen2.5-VL-7B-Instruct \
  --max-tokens 64 --data-log demo-vlm.jsonl

# Open-loop LLM:
metrum-ai-bench-llm ... --request-rate 8 --arrival constant --max-concurrency 8
EOF
}

cmd_validate() {
  local dir="${1:-${root}}"
  [[ -d "${dir}" ]] || die "campaign dir missing: ${dir} (run after sweeps)"
  find "${dir}" -name 'results.jsonl' | grep -q . || die "no results.jsonl under ${dir}"
  python3 - "${dir}" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
errors = []
for path in root.rglob("results.jsonl"):
    n = 0
    for line in path.read_text().splitlines():
        if not line.strip():
            continue
        n += 1
        rec = json.loads(line)
        if "schema_version" not in rec:
            errors.append(f"{path}: line {n} missing schema_version")
    if n == 0:
        errors.append(f"{path}: empty")
if errors:
    print("\n".join(errors), file=sys.stderr)
    sys.exit(1)
print(f"ok: jsonl records under {root} have schema_version")
PY
}

cmd_backup() {
  local dir="${1:-${root}}"
  [[ -d "${dir}" ]] || die "campaign dir missing: ${dir}"
  if [[ "${execute}" -eq 0 ]]; then
    echo "# dry-run restic backup of ${dir}"
    echo "# uses RESTIC_* from env.json; does not print secrets"
    return 0
  fi
  python3 - "${REPO_ROOT}/env.json" "${dir}" <<'PY'
import json, os, pathlib, subprocess, sys
env_path, src = pathlib.Path(sys.argv[1]), sys.argv[2]
cfg = json.loads(env_path.read_text())
host, path, password = cfg["RESTIC_REPO_HOST"], cfg["RESTIC_REPO_PATH"], cfg["RESTIC_PASSWORD"]
repo = f"sftp:{cfg.get('BACKUP_USER', 'restic')}@{host}:{path}"
env = os.environ.copy()
env["RESTIC_PASSWORD"] = password
subprocess.check_call(["restic", "-r", repo, "backup", src], env=env)
PY
}

cmd_sweep() {
  echo "# campaign ${campaign_id} lanes=${LANES} execute=${execute}"
  echo "# LLM: c=1,2,4,8 closed-loop; rate=4,8,16 constant; warmup=8 n=64 seed=7"
  echo "# VLM: c=1,2,4 streaming; warmup=8 n=32 seed=7"
  if [[ "${execute}" -eq 0 ]]; then
    echo "# dry-run: not contacting GPUs. Re-run with --execute after local gates."
    return 0
  fi
  [[ -f "${root}/instances.json" ]] || die "missing ${root}/instances.json (launch --execute first)"
  die "live sweep runner is the final stage; instances are registered. Wire per-cell benches next, then run."
}

case "${cmd}" in
  plan) cmd_plan ;;
  launch) cmd_launch ;;
  teardown) cmd_teardown ;;
  demo) cmd_demo ;;
  validate) cmd_validate "$@" ;;
  backup) cmd_backup "$@" ;;
  sweep) cmd_sweep ;;
  *) die "unknown command: ${cmd}" ;;
esac
