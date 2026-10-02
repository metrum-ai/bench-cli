# shellcheck shell=bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Metrum AI Bench CLI: shared launcher logic for scripts/live/serve/*.sh.
# Each launcher sets MODALITY, IMAGE, MODEL, SERVE_ARGS (array), optional
# DOCKER_ENV (array) and ENTRYPOINT_CMD (array, for images without a default
# entrypoint), SOURCES (array of URLs), and SUT_NOTES, then calls serve_main.
#
# Environment:
#   PORT         host port (default 8000)
#   HF_HOME      Hugging Face cache mounted into the container
#                (default ~/.cache/huggingface)
#   HF_TOKEN     passed through by name only when set; never written to disk
#   GPU_DEVICES  docker --gpus value (default "device=0")
#   SERVE_OUT    directory for sut.json and the container log
#                (default live-results/serve-<modality>)
#   READY_TIMEOUT_S  seconds to wait for /v1/models (default 1800)
#
# Subcommands: start (default) | stop | print | logs

serve_die() { echo "error: $*" >&2; exit 1; }

serve_docker_cmd() {
  local name="$1"
  local cmd=(docker run -d --name "${name}" --gpus "${GPU_DEVICES:-device=0}" --ipc=host
             -p "${PORT:-8000}:8000"
             -v "${HF_HOME:-${HOME}/.cache/huggingface}:/root/.cache/huggingface")
  [[ -n "${HF_TOKEN:-}" ]] && cmd+=(-e HF_TOKEN)
  local e
  for e in "${DOCKER_ENV[@]}"; do cmd+=(-e "${e}"); done
  cmd+=("${IMAGE}")
  if [[ ${#ENTRYPOINT_CMD[@]} -gt 0 ]]; then cmd+=("${ENTRYPOINT_CMD[@]}"); fi
  cmd+=("${MODEL}" "${SERVE_ARGS[@]}")
  printf '%q ' "${cmd[@]}"
}

serve_gpu_json() {
  if command -v nvidia-smi >/dev/null 2>&1; then
    nvidia-smi --query-gpu=name,memory.total,driver_version --format=csv,noheader,nounits -i "${GPU_INDEX:-0}" \
      | awk -F', ' '{printf "{\"model\":\"%s\",\"count\":1,\"memory_gb\":%d,\"driver\":\"NVIDIA %s\"}", $1, $2/1024, $3}'
  else
    echo '{"model":"not captured (nvidia-smi missing)","count":1,"memory_gb":null,"driver":"not captured"}'
  fi
}

serve_model_revision() {
  curl -fsS --max-time 15 "https://huggingface.co/api/models/${MODEL}/revision/main" 2>/dev/null \
    | jq -r '.sha // empty' 2>/dev/null || true
}

serve_write_sut() {
  local out="$1" docker_cmd="${2//${HOME}/\~}" gpu rev
  gpu="$(serve_gpu_json)"
  rev="$(serve_model_revision)"
  jq -n \
    --arg name "${SUT_NAME:-local GPU / ${MODALITY} live smoke}" \
    --argjson gpu "${gpu}" \
    --arg image "${IMAGE}" \
    --arg config "${docker_cmd}" \
    --arg model "${MODEL}" \
    --arg rev "${rev}" \
    --arg os "$(uname -s) $(uname -r)" \
    --arg notes "${SUT_NOTES}" \
    --arg recorded "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --args '{
      provenance: "declared",
      name: $name,
      vendor: "NVIDIA",
      gpu: {model: $gpu.model, count: $gpu.count, memory_gb: $gpu.memory_gb},
      driver_version: $gpu.driver,
      runtime: {name: ($image | split(":")[0] | split("/")[-1]), version: ($image | split(":")[1] // "unknown"), config: $config},
      model: {id: $model, revision: (if $rev == "" then null else $rev end), quantization: null},
      host_os: $os,
      notes: $notes,
      extra: {launcher_sources: ($ARGS.positional | join(" ")), recorded_at_utc: $recorded, image: $image}
    }' "${SOURCES[@]}" >"${out}"
}

serve_main() {
  local sub="${1:-start}"
  local name="metrum-live-${MODALITY}"
  local out="${SERVE_OUT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)/live-results/serve-${MODALITY}}"
  local docker_cmd
  docker_cmd="$(serve_docker_cmd "${name}")"
  case "${sub}" in
    print)
      echo "${docker_cmd}"
      ;;
    stop)
      docker rm -f "${name}" >/dev/null 2>&1 || true
      echo "stopped ${name}"
      ;;
    logs)
      docker logs "${name}"
      ;;
    start)
      command -v docker >/dev/null || serve_die "docker is required"
      command -v jq >/dev/null || serve_die "jq is required"
      mkdir -p "${out}"
      docker rm -f "${name}" >/dev/null 2>&1 || true
      echo "# ${docker_cmd}" >&2
      eval "${docker_cmd}" >/dev/null
      serve_write_sut "${out}/sut.json" "${docker_cmd}"
      local url="http://127.0.0.1:${PORT:-8000}/v1/models" deadline=$((SECONDS + ${READY_TIMEOUT_S:-1800}))
      until curl -fsS -o /dev/null --max-time 5 "${url}"; do
        if ! docker ps -q -f "name=^${name}$" | grep -q .; then
          docker logs "${name}" >"${out}/server.log" 2>&1 || true
          serve_die "${name} exited before ready; see ${out}/server.log"
        fi
        (( SECONDS < deadline )) || serve_die "timed out waiting for ${url}"
        sleep 10
      done
      echo "ready ${MODALITY} ${MODEL} on port ${PORT:-8000}; SUT at ${out}/sut.json"
      ;;
    *) serve_die "unknown subcommand: ${sub} (start|stop|print|logs)" ;;
  esac
}
