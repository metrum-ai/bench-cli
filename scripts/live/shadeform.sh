#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Shadeform helpers for optional live GPU smoke tests.
# create is dry-run by default; pass --execute to POST.
# Always trap DELETE on exit when holding a real instance (see README).
#
# Binary names (after hygiene rename): metrum-ai-bench-cli-llm, metrum-ai-bench-cli-vlm

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
API_BASE="${SHADEFORM_API_BASE:-https://api.shadeform.ai/v1}"
ENV_JSON="${ENV_JSON:-${REPO_ROOT}/env.json}"
RESULTS_DIR="${RESULTS_DIR:-${REPO_ROOT}/live-results}"

# Preference order: RTX 6000 Pro Blackwell Server Edition → B200 → H200 → H100 → L40S
# Matches Shadeform gpu_type values; consumer SKUs are skipped in pick_score.
PREFERRED_GPU_TYPES=(
  "RTXPro6000"
  "B200"
  "H200"
  "H100"
  "H100_nvl"
  "L40S"
)

LLM_MODEL="${LLM_MODEL:-Qwen/Qwen2.5-7B-Instruct}"
VLM_MODEL="${VLM_MODEL:-Qwen/Qwen2.5-VL-7B-Instruct}"
VLLM_IMAGE="${VLLM_IMAGE:-vllm/vllm-openai:latest}"
SGLANG_IMAGE="${SGLANG_IMAGE:-lmsysorg/sglang}"

usage() {
  cat <<'EOF'
Usage: shadeform.sh <subcommand> [options]

Subcommands:
  types              List available preferred GPU types
  pick               Print one preferred available type as JSON
  create             Build create payload (dry-run unless --execute)
  wait <id>          Poll until instance is active; print IP
  delete <id>        Delete instance (POST .../delete)
  run-llm            Run metrum-ai-bench-cli-llm (modest request count)
  run-vlm            Run metrum-ai-bench-cli-vlm (modest request count)

create options:
  --engine vllm|sglang   Docker engine (default: vllm)
  --modality llm|vlm     Model choice (default: llm)
  --name NAME            Instance name (default: metrum-live-YYYYMMDD-HHMMSS)
  --cloud CLOUD          Override cloud from pick
  --region REGION        Override region from pick
  --type TYPE            Override shade_instance_type from pick
  --execute              Actually POST /instances/create (default: dry-run)

Environment:
  SHADEFORM_API_KEY      Preferred; else read from env.json via jq
  ENV_JSON               Path to env.json (default: <repo>/env.json)
  RESULTS_DIR            Output dir for run-* (default: <repo>/live-results)

Always register: trap 'shadeform.sh delete "$INSTANCE_ID"' EXIT
EOF
}

die() {
  echo "error: $*" >&2
  exit 1
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

load_api_key() {
  if [[ -n "${SHADEFORM_API_KEY:-}" ]]; then
    return 0
  fi
  require_cmd jq
  [[ -f "${ENV_JSON}" ]] || die "SHADEFORM_API_KEY unset and ${ENV_JSON} missing"
  SHADEFORM_API_KEY="$(jq -r '.SHADEFORM_API_KEY // empty' "${ENV_JSON}")"
  [[ -n "${SHADEFORM_API_KEY}" && "${SHADEFORM_API_KEY}" != "null" ]] \
    || die "SHADEFORM_API_KEY not found in ${ENV_JSON}"
}

api() {
  local method="$1"
  local path="$2"
  shift 2
  load_api_key
  require_cmd curl
  local body status
  body="$(mktemp)"
  status="$(curl --retry 3 --retry-all-errors -sS -o "${body}" -w '%{http_code}' -X "${method}" \
    -H "X-API-KEY: ${SHADEFORM_API_KEY}" \
    -H "Content-Type: application/json" \
    -H "Accept: application/json" \
    "${API_BASE}${path}" \
    "$@")"
  if [[ ! "${status}" =~ ^2 ]]; then
    echo "Shadeform API ${method} ${path} returned HTTP ${status}: $(<"${body}")" >&2
    rm -f "${body}"
    return 1
  fi
  cat "${body}"
  rm -f "${body}"
}

fetch_types_json() {
  require_cmd jq
  api GET "/instances/types"
}

# Emit TSV: rank  num_gpus  price  cloud  type  region  gpu_type  hourly_price
list_available_preferred() {
  local types_json
  types_json="$(fetch_types_json)"
  jq -r --argjson prefer "$(printf '%s\n' "${PREFERRED_GPU_TYPES[@]}" | jq -R . | jq -s .)" '
    def rank($g):
      if ($g == "RTX4090" or $g == "RTX5090" or $g == "RTX4000Ada"
          or $g == "RTX6000" or $g == "RTX6000Ada" or $g == "A10"
          or $g == "A16" or $g == "CPU") then empty
      else ($prefer | index($g)) end;
    .instance_types[]
    | . as $it
    | (rank($it.gpu_type)) as $r
    | select($r != null)
    | ($it.availability // [])[]
    | select(.available == true)
    | [
        $r,
        ($it.num_gpus // 1),
        ($it.hourly_price // 999999),
        $it.cloud,
        $it.shade_instance_type,
        .region,
        $it.gpu_type,
        ($it.hourly_price // 0)
      ]
    | @tsv
  ' <<<"${types_json}" | sort -t$'\t' -k1,1n -k2,2n -k3,3n
}

cmd_types() {
  echo "# preferred available (rank cloud type region gpu_type price_cents_per_hour num_gpus)"
  local line rank ngpus price cloud typ region gpu
  while IFS=$'\t' read -r rank ngpus price cloud typ region gpu _price; do
    [[ -n "${rank:-}" ]] || continue
    printf 'rank=%s cloud=%s type=%s region=%s gpu=%s price=%s num_gpus=%s\n' \
      "${rank}" "${cloud}" "${typ}" "${region}" "${gpu}" "${price}" "${ngpus}"
  done < <(list_available_preferred)
}

cmd_pick() {
  require_cmd jq
  local best
  best="$(list_available_preferred | head -n1 || true)"
  [[ -n "${best}" ]] || die "no preferred GPU types currently available"
  local rank ngpus price cloud typ region gpu
  IFS=$'\t' read -r rank ngpus price cloud typ region gpu _price <<<"${best}"
  jq -n \
    --arg cloud "${cloud}" \
    --arg type "${typ}" \
    --arg region "${region}" \
    --arg gpu_type "${gpu}" \
    --argjson num_gpus "${ngpus}" \
    --argjson hourly_price "${price}" \
    --argjson rank "${rank}" \
    '{cloud:$cloud, shade_instance_type:$type, region:$region, gpu_type:$gpu_type,
      num_gpus:$num_gpus, hourly_price:$hourly_price, rank:$rank}'
}

build_docker_config() {
  local engine="$1"
  local modality="$2"
  local model="${3:-}"
  local extra_args="${4:-}"
  local image args
  if [[ -z "${model}" ]]; then
    if [[ "${modality}" == "vlm" ]]; then
      model="${VLM_MODEL}"
    elif [[ "${modality}" == "asr" ]]; then
      model="${ASR_MODEL:-openai/whisper-large-v3}"
    else
      model="${LLM_MODEL}"
    fi
  fi
  case "${engine}" in
    vllm)
      image="${VLLM_IMAGE}"
      args="--model ${model} --host 0.0.0.0 --port 8000"
      ;;
    sglang)
      image="${SGLANG_IMAGE}"
      args="python3 -m sglang.launch_server --model-path ${model} --host 0.0.0.0 --port 8000"
      ;;
    *)
      die "unknown engine: ${engine} (expected vllm|sglang)"
      ;;
  esac
  if [[ -n "${extra_args}" ]]; then
    args="${args} ${extra_args}"
  fi
  jq -n \
    --arg image "${image}" \
    --arg args "${args}" \
    '{
       type: "docker",
       docker_configuration: {
         image: $image,
         args: $args,
         shared_memory_in_gb: 32,
         port_mappings: [{host_port: 80, container_port: 8000}]
       }
     }'
}

print_trap_hint() {
  cat <<'EOF'

# Teardown - always trap DELETE on exit when holding a real instance:
# INSTANCE_ID="<id-from-create>"
# cleanup() { [[ -n "${INSTANCE_ID}" ]] && ./scripts/live/shadeform.sh delete "${INSTANCE_ID}" || true; }
# trap cleanup EXIT
EOF
}

cmd_create() {
  require_cmd jq
  local engine="vllm"
  local modality="llm"
  local name=""
  local cloud="" region="" typ=""
  local model=""
  local extra_args=""
  local execute=0

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --engine) engine="$2"; shift 2 ;;
      --modality) modality="$2"; shift 2 ;;
      --name) name="$2"; shift 2 ;;
      --cloud) cloud="$2"; shift 2 ;;
      --region) region="$2"; shift 2 ;;
      --type) typ="$2"; shift 2 ;;
      --model) model="$2"; shift 2 ;;
      --extra-args) extra_args="$2"; shift 2 ;;
      --execute) execute=1; shift ;;
      --dry-run) execute=0; shift ;;
      -h|--help) usage; return 0 ;;
      *) die "unknown create option: $1" ;;
    esac
  done

  case "${modality}" in
    llm|vlm|asr) ;;
    *) die "modality must be llm, vlm, or asr" ;;
  esac

  if [[ -z "${name}" ]]; then
    name="metrum-live-$(date -u +%Y%m%d-%H%M%S)"
  fi

  local pick_json
  if [[ -z "${cloud}" || -z "${region}" || -z "${typ}" ]]; then
    pick_json="$(cmd_pick)"
    cloud="${cloud:-$(jq -r .cloud <<<"${pick_json}")}"
    region="${region:-$(jq -r .region <<<"${pick_json}")}"
    typ="${typ:-$(jq -r .shade_instance_type <<<"${pick_json}")}"
  fi

  # Auto tensor-parallel for multi-GPU SKUs when caller did not set --extra-args.
  if [[ -z "${extra_args}" && "${typ}" =~ [xX]2$ ]]; then
    extra_args="--tensor-parallel-size 2"
  elif [[ -z "${extra_args}" && "${typ}" =~ [xX]4$ ]]; then
    extra_args="--tensor-parallel-size 4"
  elif [[ -z "${extra_args}" && "${typ}" =~ [xX]8$ ]]; then
    extra_args="--tensor-parallel-size 8"
  fi

  local launch
  launch="$(build_docker_config "${engine}" "${modality}" "${model}" "${extra_args}")"

  local payload
  payload="$(jq -n \
    --arg cloud "${cloud}" \
    --arg region "${region}" \
    --arg type "${typ}" \
    --arg name "${name}" \
    --arg ssh_key_id "${SHADEFORM_SSH_KEY_ID:-}" \
    --argjson launch "${launch}" \
    '{
       cloud: $cloud,
       region: $region,
       shade_instance_type: $type,
       shade_cloud: true,
       name: $name,
       launch_configuration: $launch
     }
     | if $ssh_key_id == "" then . else . + {ssh_key_id: $ssh_key_id} end')"

  echo "# create payload (engine=${engine} modality=${modality})"
  echo "${payload}" | jq .
  print_trap_hint

  if [[ "${execute}" -eq 0 ]]; then
    echo
    echo "# dry-run: not POSTing. Re-run with --execute to create."
    echo "curl -sS -X POST \\"
    echo "  -H \"X-API-KEY: \$SHADEFORM_API_KEY\" \\"
    echo "  -H \"Content-Type: application/json\" \\"
    echo "  -d @payload.json \\"
    echo "  ${API_BASE}/instances/create"
    return 0
  fi

  echo "# --execute: POSTing /instances/create" >&2
  local resp
  resp="$(api POST "/instances/create" -d "${payload}")"
  echo "${resp}" | jq .
  local id
  id="$(jq -r '.id // .instance_id // empty' <<<"${resp}")"
  if [[ -n "${id}" ]]; then
    echo "# created id=${id}" >&2
    echo "# remember: trap cleanup EXIT → shadeform.sh delete ${id}" >&2
  fi
}

cmd_wait() {
  require_cmd jq
  local id="${1:-}"
  [[ -n "${id}" ]] || die "wait requires <instance-id>"
  local max_attempts="${WAIT_MAX_ATTEMPTS:-60}"
  local delay="${WAIT_DELAY_SECONDS:-10}"
  local i status ip
  for ((i = 1; i <= max_attempts; i++)); do
    local resp
    if ! resp="$(api GET "/instances/${id}/info")" || ! jq -e . >/dev/null 2>&1 <<<"${resp}"; then
      echo "# wait attempt ${i}/${max_attempts}: transient invalid API response" >&2
      sleep "${delay}"
      continue
    fi
    status="$(jq -r '.status // .instance.status // empty' <<<"${resp}")"
    ip="$(jq -r '
      .ip // .instance.ip // .public_ip // .instance.public_ip // empty
    ' <<<"${resp}")"
    echo "# wait attempt ${i}/${max_attempts} status=${status:-unknown} ip=${ip:-none}" >&2
    case "${status}" in
      active|running|ready)
        if [[ -n "${ip}" && "${ip}" != "null" ]]; then
          echo "${ip}"
          return 0
        fi
        # Still active but IP not yet assigned
        ;;
      failed|error|deleted|terminated)
        echo "${resp}" | jq . >&2
        die "instance ${id} entered terminal status: ${status}"
        ;;
    esac
    sleep "${delay}"
  done
  die "timed out waiting for instance ${id}"
}

cmd_delete() {
  local id="${1:-}"
  [[ -n "${id}" ]] || die "delete requires <instance-id>"
  echo "# deleting instance ${id}" >&2
  local resp
  resp="$(api POST "/instances/${id}/delete" -d '{}')"
  if [[ -n "${resp}" ]]; then
    echo "${resp}" | jq . 2>/dev/null || echo "${resp}"
  else
    echo "# delete requested (empty body)"
  fi
}

resolve_bench_bin() {
  local want="$1" # metrum-ai-bench-cli-llm | metrum-ai-bench-cli-vlm
  local legacy="${want/metrum-ai-bench-cli/metrumbench}"
  if command -v "${want}" >/dev/null 2>&1; then
    echo "${want}"
    return 0
  fi
  if command -v "${legacy}" >/dev/null 2>&1; then
    echo "${legacy}"
    return 0
  fi
  local candidate
  for candidate in \
    "${REPO_ROOT}/target/release/${want}" \
    "${REPO_ROOT}/target/debug/${want}" \
    "${REPO_ROOT}/target/release/${legacy}" \
    "${REPO_ROOT}/target/debug/${legacy}"; do
    if [[ -x "${candidate}" ]]; then
      echo "${candidate}"
      return 0
    fi
  done
  die "binary not found: ${want} (or legacy ${legacy}); build the crate first"
}

cmd_run_llm() {
  local url=""
  local num_requests=4
  local concurrency=1
  local model="${LLM_MODEL}"
  local out_dir="${RESULTS_DIR}/llm-$(date -u +%Y%m%d-%H%M%S)"

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --url) url="$2"; shift 2 ;;
      --num-requests) num_requests="$2"; shift 2 ;;
      --concurrency) concurrency="$2"; shift 2 ;;
      --model) model="$2"; shift 2 ;;
      --out-dir) out_dir="$2"; shift 2 ;;
      *) die "unknown run-llm option: $1" ;;
    esac
  done
  [[ -n "${url}" ]] || die "run-llm requires --url"

  mkdir -p "${out_dir}"
  local prompts="${out_dir}/prompts.jsonl"
  if [[ ! -f "${prompts}" ]]; then
    printf '%s\n' \
      '{"prompt":"Say hello in one short sentence."}' \
      '{"prompt":"What is 2+2? Reply with one number."}' \
      '{"prompt":"Name a primary color."}' \
      '{"prompt":"Reply with the word ok."}' \
      >"${prompts}"
  fi

  local bin
  bin="$(resolve_bench_bin metrum-ai-bench-cli-llm)"
  echo "# running ${bin} → ${out_dir}" >&2
  "${bin}" \
    --url "${url}" \
    --num-requests "${num_requests}" \
    --concurrency "${concurrency}" \
    --mode chat \
    --streaming \
    --prompts "${prompts}" \
    --model "${model}" \
    --data-log "${out_dir}/results.jsonl" \
    --max-tokens 64 \
    --temperature 0.1 \
    --debug-log "${out_dir}/debug.log" \
    --error-log "${out_dir}/error.log" \
    --scenario "shadeform-live-llm" \
    --api-key "none" \
    --log-level info
}

cmd_run_vlm() {
  local url=""
  local num_requests=2
  local concurrency=1
  local model="${VLM_MODEL}"
  local out_dir="${RESULTS_DIR}/vlm-$(date -u +%Y%m%d-%H%M%S)"
  local prompts=""

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --url) url="$2"; shift 2 ;;
      --num-requests) num_requests="$2"; shift 2 ;;
      --concurrency) concurrency="$2"; shift 2 ;;
      --model) model="$2"; shift 2 ;;
      --out-dir) out_dir="$2"; shift 2 ;;
      --prompts) prompts="$2"; shift 2 ;;
      *) die "unknown run-vlm option: $1" ;;
    esac
  done
  [[ -n "${url}" ]] || die "run-vlm requires --url"
  [[ -n "${prompts}" ]] || die "run-vlm requires --prompts <jsonl with image refs>"

  mkdir -p "${out_dir}"
  local bin
  bin="$(resolve_bench_bin metrum-ai-bench-cli-vlm)"
  echo "# running ${bin} → ${out_dir}" >&2
  "${bin}" \
    --url "${url}" \
    --num-requests "${num_requests}" \
    --concurrency "${concurrency}" \
    --prompts "${prompts}" \
    --model "${model}" \
    --data-log "${out_dir}/results.jsonl" \
    --max-tokens 64 \
    --temperature 0.1 \
    --debug-log "${out_dir}/debug.log" \
    --error-log "${out_dir}/error.log" \
    --scenario "shadeform-live-vlm" \
    --api-key "none" \
    --streaming \
    --log-level info
}

main() {
  local cmd="${1:-}"
  [[ -n "${cmd}" ]] || { usage; exit 1; }
  shift || true
  case "${cmd}" in
    types) cmd_types "$@" ;;
    pick) cmd_pick "$@" ;;
    create) cmd_create "$@" ;;
    wait) cmd_wait "$@" ;;
    delete) cmd_delete "$@" ;;
    run-llm) cmd_run_llm "$@" ;;
    run-vlm) cmd_run_vlm "$@" ;;
    -h|--help|help) usage ;;
    *) die "unknown subcommand: ${cmd}" ;;
  esac
}

main "$@"
