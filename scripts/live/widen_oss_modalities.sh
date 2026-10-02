#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Metrum AI Bench CLI widen smoke on one Shadeform GPU VM (plain VM + SSH,
# not the single-container docker launch), so one instance can serve LLM,
# VLM, and ASR in turn next to the all-smi exporter.
#
# Subcommands:
#   up        create the VM and HOLD it in the foreground until `down`, the
#             TTL, or a signal; an EXIT trap always deletes the instance.
#             Run it in the background: `widen_oss_modalities.sh up &`.
#   down      ask the holder to delete the instance and exit
#   status    print instance id, status, and price from the state dir
#   ssh CMD   run CMD on the VM
#   push      rsync built binaries, scripts, fixtures, and docs/telemetry
#   fetch SRC DST  rsync a remote path back
#
# Environment:
#   ENV_JSON            Shadeform key file (default <repo>/env.json); the key
#                       is read with jq and never printed
#   SHADEFORM_SSH_KEY_ID  registered key matching SSH_IDENTITY (required)
#   SSH_IDENTITY        private key path (default ~/.ssh/id_ed25519)
#   GPU_TYPE / NUM_GPUS  default H100 / 1
#   SHADE_CLOUD SHADE_REGION SHADE_TYPE SHADE_OS  override the cheapest pick
#   TTL_HOURS           hard lifetime for the holder (default 4)
#   WIDEN_STATE         state dir (default live-results/widen-state)
#   BENCH_BIN_DIR       local binaries to push (default target/release)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
ENV_JSON="${ENV_JSON:-${REPO_ROOT}/env.json}"
API="${SHADEFORM_API_BASE:-https://api.shadeform.ai/v1}"
STATE="${WIDEN_STATE:-${REPO_ROOT}/live-results/widen-state}"
SSH_IDENTITY="${SSH_IDENTITY:-${HOME}/.ssh/id_ed25519}"
REMOTE_ROOT="${REMOTE_ROOT:-/opt/metrum-bench}"
mkdir -p "${STATE}"

die() { echo "error: $*" >&2; exit 1; }
log() { echo "# $(date -u +%H:%M:%SZ) $*" >&2; }

api() { # METHOD PATH [curl args]; key never echoed
  local method="$1" path="$2" key
  shift 2
  key="$(jq -r '.SHADEFORM_API_KEY // empty' "${ENV_JSON}")"
  [[ -n "${key}" ]] || die "SHADEFORM_API_KEY missing in ${ENV_JSON}"
  curl --retry 3 --retry-all-errors -fsS -X "${method}" \
    -H "X-API-KEY: ${key}" -H "Content-Type: application/json" \
    "${API}${path}" "$@"
}

state_get() { jq -r ".$1 // empty" "${STATE}/instance.json" 2>/dev/null || true; }

ssh_opts() {
  printf '%s\n' -i "${SSH_IDENTITY}" -o StrictHostKeyChecking=accept-new \
    -o UserKnownHostsFile="${STATE}/known_hosts" -o ConnectTimeout=15 \
    -o ServerAliveInterval=30 -p "$(state_get ssh_port)"
}

remote() {
  local opts
  mapfile -t opts < <(ssh_opts)
  ssh "${opts[@]}" "$(state_get ssh_user)@$(state_get ip)" "$@"
}

pick() {
  local gpu="${GPU_TYPE:-H100}" n="${NUM_GPUS:-1}"
  api GET "/instances/types?gpu_type=${gpu}&num_gpus=${n}&available=true" | jq -c --arg g "${gpu}" --argjson n "${n}" '
    [.instance_types[] | select(.gpu_type == $g and .num_gpus == $n)
     | . as $t | ($t.availability // [])[] | select(.available)
     | {cloud: $t.cloud, region: .region, type: $t.shade_instance_type,
        price: $t.hourly_price,
        os: (($t.configuration.os_options // []) | map(select(test("ubuntu24.04_cuda13"))) | first)}]
    | sort_by(.price) | first'
}

cmd_up() {
  [[ -n "${SHADEFORM_SSH_KEY_ID:-}" ]] || die "set SHADEFORM_SSH_KEY_ID to the key registered for ${SSH_IDENTITY}"
  [[ ! -s "${STATE}/instance.json" ]] || die "state already holds $(state_get id); run down first"
  rm -f "${STATE}/stop"
  local p cloud region typ os name
  p="$(pick)"
  [[ -n "${p}" && "${p}" != null ]] || die "no available ${GPU_TYPE:-H100} x${NUM_GPUS:-1}"
  cloud="${SHADE_CLOUD:-$(jq -r .cloud <<<"${p}")}"
  region="${SHADE_REGION:-$(jq -r .region <<<"${p}")}"
  typ="${SHADE_TYPE:-$(jq -r .type <<<"${p}")}"
  os="${SHADE_OS:-$(jq -r '.os // empty' <<<"${p}")}"
  name="metrum-widen-$(date -u +%Y%m%d-%H%M%S)"
  log "creating ${typ} on ${cloud}/${region} os=${os:-default} price_cents_per_hour=$(jq -r .price <<<"${p}")"
  local payload resp id
  payload="$(jq -n --arg c "${cloud}" --arg r "${region}" --arg t "${typ}" --arg n "${name}" \
    --arg k "${SHADEFORM_SSH_KEY_ID}" --arg os "${os}" \
    '{cloud:$c, region:$r, shade_instance_type:$t, shade_cloud:true, name:$n, ssh_key_id:$k}
     + (if $os == "" then {} else {os:$os} end)')"
  resp="$(api POST /instances/create -d "${payload}")"
  id="$(jq -r '.id // empty' <<<"${resp}")"
  [[ -n "${id}" ]] || die "create returned no id"
  jq -n --arg id "${id}" --arg name "${name}" --argjson pick "${p}" \
    --arg created "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    '{id:$id, name:$name, pick:$pick, created_at_utc:$created}' >"${STATE}/instance.json"

  # From here on the instance always gets deleted when this process exits.
  # The trap runs after cmd_up returns, when its locals are gone, so the id
  # lives in a global (a local here once made the trap fail with
  # "id: unbound variable" and leak the instance).
  WIDEN_INSTANCE_ID="${id}"
  cleanup() {
    log "deleting instance ${WIDEN_INSTANCE_ID}"
    api POST "/instances/${WIDEN_INSTANCE_ID}/delete" -d '{}' >/dev/null \
      || log "delete call failed; delete ${WIDEN_INSTANCE_ID} by hand"
    jq --arg t "$(date -u +%Y-%m-%dT%H:%M:%SZ)" '.deleted_at_utc = $t' "${STATE}/instance.json" \
      >"${STATE}/instance.done.json" 2>/dev/null || true
    rm -f "${STATE}/instance.json"
  }
  trap cleanup EXIT
  trap 'exit 130' INT TERM HUP

  local i info status
  for ((i = 1; i <= 90; i++)); do
    info="$(api GET "/instances/${id}/info" || echo '{}')"
    status="$(jq -r '.status // empty' <<<"${info}")"
    log "wait ${i}/90 status=${status:-unknown}"
    if [[ "${status}" == active ]] && [[ -n "$(jq -r '.ip // empty' <<<"${info}")" ]]; then
      jq --argjson info "${info}" '. + {ip: $info.ip, ssh_user: ($info.ssh_user // "shadeform"),
          ssh_port: ($info.ssh_port // 22), status: $info.status,
          hourly_price: $info.hourly_price, cloud: $info.cloud, region: $info.region}' \
        "${STATE}/instance.json" >"${STATE}/i.tmp" && mv "${STATE}/i.tmp" "${STATE}/instance.json"
      break
    fi
    [[ "${status}" =~ ^(error|deleted|failed)$ ]] && die "instance entered ${status}"
    sleep 20
  done
  [[ -n "$(state_get ip)" ]] || die "instance never became active"
  for ((i = 1; i <= 30; i++)); do
    remote true 2>/dev/null && break
    sleep 10
  done
  remote true || die "ssh never came up"
  touch "${STATE}/ready"
  log "ready: $(state_get ssh_user)@<instance ip> (see ${STATE}/instance.json); holding"

  local deadline=$((SECONDS + ${TTL_HOURS:-4} * 3600))
  while [[ ! -e "${STATE}/stop" ]] && (( SECONDS < deadline )); do sleep 15; done
  (( SECONDS < deadline )) || log "TTL ${TTL_HOURS:-4}h reached"
}

cmd_down() { touch "${STATE}/stop"; log "stop requested"; }

cmd_status() {
  [[ -s "${STATE}/instance.json" ]] || { echo "no instance held"; return 0; }
  api GET "/instances/$(state_get id)/info" | jq '{id, status, cloud, region, shade_instance_type, hourly_price}'
}

cmd_push() {
  local bin="${BENCH_BIN_DIR:-${REPO_ROOT}/target/release}" opts
  mapfile -t opts < <(ssh_opts)
  local rsh="ssh ${opts[*]}" dest
  dest="$(state_get ssh_user)@$(state_get ip)"
  remote "sudo mkdir -p ${REMOTE_ROOT}/bin && sudo chown -R \$(id -u):\$(id -g) ${REMOTE_ROOT}"
  rsync -az -e "${rsh}" "${bin}"/metrum-ai-bench-cli "${bin}"/metrum-ai-bench-cli-{llm,vlm,asr,imagegen,strategic,prompts} \
    "${dest}:${REMOTE_ROOT}/bin/"
  rsync -az -e "${rsh}" --relative \
    ./scripts/live ./test-data ./docs/telemetry ./examples \
    "${dest}:${REMOTE_ROOT}/"
}

cmd_fetch() { # SRC (remote) DST (local)
  local opts
  mapfile -t opts < <(ssh_opts)
  rsync -az -e "ssh ${opts[*]}" "$(state_get ssh_user)@$(state_get ip):$1" "$2"
}

cd "${REPO_ROOT}"
sub="${1:-}"; shift || true
case "${sub}" in
  up) cmd_up ;;
  down) cmd_down ;;
  status) cmd_status ;;
  ssh) remote "$@" ;;
  push) cmd_push ;;
  fetch) cmd_fetch "$@" ;;
  *) sed -n '5,30p' "$0" >&2; exit 2 ;;
esac
