#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
# Shared GPU-lane lifecycle; source from a lane rather than executing directly.
set -euo pipefail
CERT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

cert_parse() {
  python3 -c 'import sys; sys.version_info >= (3, 12) or sys.exit("certification harness requires Python 3.12 or newer")'
  CERT_LANE=$1; shift
  CERT_CANDIDATE='' CERT_BASELINE='' CERT_MANIFEST='' CERT_OUT='' CERT_AUX_OWNER=''
  CERT_PERF_FINALIZE=0
  [[ "$CERT_LANE" != P1 && "$CERT_LANE" != P2 ]] || CERT_PERF_FINALIZE=1
  while (($#)); do
    case "$1" in
      --plan) python3 "$CERT_DIR/lane.py" "$CERT_LANE" --plan; return 10 ;;
      --candidate|--baseline|--manifest|--out)
        (($# >= 2)) || { echo "missing argument for $1" >&2; return 2; }
        case "$1" in
          --candidate) CERT_CANDIDATE=$2 ;;
          --baseline) CERT_BASELINE=$2 ;;
          --manifest) CERT_MANIFEST=$2 ;;
          --out) CERT_OUT=$2 ;;
        esac
        shift 2 ;;
      *) echo "unknown argument: $1" >&2; return 2 ;;
    esac
  done
  [[ -n "$CERT_CANDIDATE" && -n "$CERT_BASELINE" && -n "$CERT_MANIFEST" && -n "$CERT_OUT" ]] || {
    echo 'required: --candidate --baseline --manifest --out' >&2; return 2;
  }
  [[ -n "${CUDA_VISIBLE_DEVICES:-}" ]] || { echo 'select exactly one CUDA_VISIBLE_DEVICES GPU' >&2; return 2; }
  CERT_ATTEMPT="${CERT_ATTEMPT:-1}"
  [[ "$CERT_ATTEMPT" =~ ^[1-9][0-9]*$ ]] || { echo 'CERT_ATTEMPT must be a positive integer' >&2; return 2; }
  export CERT_ATTEMPT
  CERT_CANDIDATE="$(realpath "$CERT_CANDIDATE")"
  CERT_BASELINE="$(realpath "$CERT_BASELINE")"
  CERT_MANIFEST="$(realpath "$CERT_MANIFEST")"
  # Manifest validation occurs before allocating or starting any process.
  CERT_SHA="$(cert_manifest candidate_sha)"
  CERT_IMAGE="$(cert_manifest image)"
  CERT_MODEL="$(cert_manifest model)"
  CERT_REVISION="$(cert_manifest revision)"
  CERT_PORT="$(cert_manifest port)"
  mkdir -- "$CERT_OUT" || { echo 'output directory already exists or cannot be created' >&2; return 1; }
  CERT_OUT="$(realpath "$CERT_OUT")"
  mkdir "$CERT_OUT/evidence" "$CERT_OUT/results"
  CERT_CONTAINER="cert-${CERT_LANE,,}-$(python3 -c 'import secrets; print(secrets.token_hex(12))')"
  CERT_STARTED=0
  CERT_START_UTC="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  export CERT_SHA CERT_START_UTC CERT_MANIFEST CERT_CANDIDATE CERT_BASELINE
  CERT_MANIFEST_SHA="$(sha256sum "$CERT_MANIFEST" | cut -d' ' -f1)"
  trap 'cert_on_exit "$?"' EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  nvidia-smi --id="$CUDA_VISIBLE_DEVICES" --query-gpu=name,driver_version --format=csv,noheader > "$CERT_OUT/evidence/gpu.csv"
  sha256sum "$CERT_CANDIDATE" "$CERT_BASELINE" > "$CERT_OUT/evidence/binary-hashes.txt"
}

cert_manifest() {
  python3 "$CERT_DIR/manifest_tool.py" --manifest "$CERT_MANIFEST" --lane "$CERT_LANE" \
    --candidate "$CERT_CANDIDATE" --baseline "$CERT_BASELINE" --field "$1"
}

cert_start_server() {
  local -a flags=() gpu_args=(--gpus "device=${CUDA_VISIBLE_DEVICES}") token_args=() cache_args=() owner_args=() offline_args=()
  [[ -z "${CERT_CONTAINER_OWNER:-}" ]] || owner_args=(--label "metrum.cert.owner=$CERT_CONTAINER_OWNER")
  [[ "${CERT_OFFLINE:-0}" != 1 ]] || offline_args=(--pull=never)
  local now_hash
  now_hash="$(sha256sum "$CERT_MANIFEST" | cut -d' ' -f1)"
  [[ "$now_hash" == "$CERT_MANIFEST_SHA" ]] || { echo 'manifest changed during run' >&2; return 1; }
  # Write to a file first so a failing producer cannot be hidden by process substitution.
  cert_manifest server_flags > "$CERT_OUT/evidence/server-flags.nul"
  mapfile -d '' -t flags < "$CERT_OUT/evidence/server-flags.nul"
  [[ -z "${HF_TOKEN:-}" ]] || token_args=(--env HF_TOKEN)
  # Mount only the hub cache, never the parent directory that may contain tokens.
  if [[ -n "${CERT_HF_HUB_CACHE:-}" ]]; then
    [[ -d "$CERT_HF_HUB_CACHE" ]] || { echo 'configured Hugging Face hub cache is missing' >&2; return 1; }
    cache_args=(--mount "type=bind,src=$(realpath "$CERT_HF_HUB_CACHE"),dst=/root/.cache/huggingface/hub" --env HF_HUB_OFFLINE=1)
  fi
  # Arm cleanup before launch so an interrupted docker client cannot orphan the container.
  CERT_STARTED=1
  docker run -d --name "$CERT_CONTAINER" "${offline_args[@]}" "${owner_args[@]}" "${gpu_args[@]}" --ipc=host \
    --env CUDA_VISIBLE_DEVICES=0 "${token_args[@]}" "${cache_args[@]}" -p "127.0.0.1:${CERT_PORT}:8000" \
    "$CERT_IMAGE" --model "$CERT_MODEL" --revision "$CERT_REVISION" \
    --port 8000 "${flags[@]}" > "$CERT_OUT/evidence/container-id.txt"
  local attempt
  for ((attempt=0; attempt<900; attempt++)); do
    if curl --fail --silent --max-time 5 "http://localhost:${CERT_PORT}/v1/models" > "$CERT_OUT/evidence/models.json"; then
      return 0
    fi
    docker inspect "$CERT_CONTAINER" --format '{{.State.Running}}' | grep -qx true || return 1
    sleep 2
  done
  echo 'server readiness timeout' >&2
  return 1
}

cert_server_logs() {
  ((CERT_STARTED == 1)) || return 1
  docker logs "$CERT_CONTAINER" > "$CERT_OUT/evidence/server.log" 2>&1
}

cert_run() {
  "$@" &
  CERT_RUNNER_PID=$!
  local rc=0
  wait "$CERT_RUNNER_PID" || rc=$?
  CERT_RUNNER_PID=''
  return "$rc"
}

cert_on_exit() {
  local rc=$1 original_rc=$1
  trap - EXIT INT TERM
  set +e
  if [[ -n "${CERT_RUNNER_PID:-}" ]]; then
    kill -TERM "$CERT_RUNNER_PID" 2>/dev/null
    wait "$CERT_RUNNER_PID" 2>/dev/null
  fi
  if ((CERT_STARTED == 1)); then
    local owned=1
    if [[ -n "${CERT_CONTAINER_OWNER:-}" ]]; then
      local actual_owner
      actual_owner="$(docker inspect --format '{{index .Config.Labels "metrum.cert.owner"}}' "$CERT_CONTAINER" 2>/dev/null)" || actual_owner=''
      if [[ "$actual_owner" != "$CERT_CONTAINER_OWNER" ]]; then
        echo 'owned-container label missing or mismatched; refusing removal' >&2
        owned=0
        rc=1
      fi
    fi
    if ((owned == 1)); then
      docker logs "$CERT_CONTAINER" > "$CERT_OUT/evidence/server.log" 2>&1 || {
        [[ "${CERT_PERF_FINALIZE:-0}" != 1 && "${CERT_LANE:-}" != W ]] || rc=1
      }
      if ! docker rm -f "$CERT_CONTAINER" > "$CERT_OUT/evidence/server-stop.txt" 2>&1; then
        echo 'failed to stop owned server container' >&2
        rc=1
      elif [[ -n "${CERT_CONTAINER_OWNER:-}" ]]; then
        local remaining
        remaining="$(docker ps -a --filter "label=metrum.cert.owner=$CERT_CONTAINER_OWNER" --format '{{.ID}}')" || rc=1
        [[ -z "$remaining" ]] || { echo 'owned container remains after cleanup' >&2; rc=1; }
      fi
    fi
  fi
  # A nested perf runner owns a separate labelled container. Independently
  # verify its cleanup even if the runner process was interrupted or killed.
  if [[ -n "${CERT_AUX_OWNER:-}" ]]; then
    local aux_remaining aux_name aux_owner
    aux_name="perfval-${CERT_AUX_OWNER}"
    aux_remaining="$(docker ps -a --filter "label=metrum.cert.owner=$CERT_AUX_OWNER" --format '{{.ID}}')" || rc=1
    if [[ -n "$aux_remaining" ]]; then
      echo 'performance container remained after runner; lane failed' >&2
      rc=1
      aux_owner="$(docker inspect --format '{{index .Config.Labels "metrum.cert.owner"}}' "$aux_name" 2>/dev/null)" || aux_owner=''
      if [[ "$aux_owner" == "$CERT_AUX_OWNER" ]]; then
        docker logs "$aux_name" > "$CERT_OUT/evidence/perf-emergency-server.log" 2>&1
        docker rm -f "$aux_name" > "$CERT_OUT/evidence/perf-emergency-stop.txt" 2>&1 || rc=1
        aux_remaining="$(docker ps -a --filter "label=metrum.cert.owner=$CERT_AUX_OWNER" --format '{{.ID}}')" || rc=1
        [[ -z "$aux_remaining" ]] || echo 'performance container remains after emergency cleanup' >&2
      else
        echo 'performance container owner mismatch; refusing removal' >&2
      fi
    fi
  fi
  # W collects first; external platform sign-off is a separate manual step.
  # A successful cleanup marker is needed before finalization can publish PASS.
  if [[ "${CERT_LANE:-}" == W && $rc == 0 ]]; then
    printf '{"cleanup":"verified"}\n' > "$CERT_OUT/evidence/cleanup-verified.json" || rc=1
  fi
  # Performance records are provisional until both exporter and nested server
  # cleanup have been checked. A killed shell cannot publish a passing record.
  if [[ "${CERT_PERF_FINALIZE:-0}" == 1 ]]; then
    local -a finalize_args=(--out "$CERT_OUT")
    if ((rc == 0)); then
      printf '{"cleanup":"verified"}\n' > "$CERT_OUT/evidence/cleanup-verified.json" || rc=1
    fi
    ((rc == 0)) || finalize_args+=(--fail)
    python3 "$CERT_DIR/finalize_perf.py" "${finalize_args[@]}" "${CERT_CASES[@]}" || rc=1
  fi
  local case_id failure_type=infra_failed
  [[ "$original_rc" != 130 && "$original_rc" != 143 ]] || failure_type=infra_interrupted
  for case_id in "${CERT_CASES[@]:-}"; do
    [[ -n "$case_id" ]] || continue
    if [[ ! -e "$CERT_OUT/results/${case_id}.json" ]]; then
      python3 "$CERT_DIR/failure_writer.py" --out "$CERT_OUT" --id "$case_id" --sha "$CERT_SHA" \
        --start "$CERT_START_UTC" --type "$failure_type" --reason 'lane exited before this case completed' || true
      rc=1
    fi
  done
  # Never allow otherwise-passing per-case records to mask a failed lane or cleanup.
  # The validator rejects unexpected JSON, so this sentinel blocks certification.
  if ((rc != 0)); then
    printf '{"error":"lane or cleanup failed; inspect retained evidence"}\n' > "$CERT_OUT/results/lane-error.json"
  fi
  exit "$rc"
}

# Existing plan-only lanes call this file as an entrypoint. No test is run.
if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  exec python3 "$CERT_DIR/lane.py" "$@"
fi
