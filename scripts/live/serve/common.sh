# shellcheck shell=bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Metrum AI Bench CLI: shared launcher logic for scripts/live/serve/*.sh.
# Each launcher sets MODALITY, IMAGE, DEFAULT_MODEL, MODEL, SERVE_ARGS
# (array), optional DOCKER_ENV (array) and ENTRYPOINT_CMD (array, for images
# without a default entrypoint), SOURCES (array of URLs), and SUT_NOTES, then
# calls serve_main. SUT_NOTES and SOURCES describe DEFAULT_MODEL only.
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
#   SERVE_ARGS_OVERRIDE / SOURCES_OVERRIDE / SUT_NOTES_OVERRIDE  replace the
#                launcher's flags, source URLs, and SUT notes (with MODEL=...).
#                When MODEL differs from DEFAULT_MODEL, SUT_NOTES_OVERRIDE is
#                required: start and sut exit with an error naming it, because
#                the launcher notes were researched for DEFAULT_MODEL only.
#   QUANTIZATION SUT model.quantization (use "none" for null). When unset it
#                comes from --quantization/-q in SERVE_ARGS, then from a
#                quantizer token in the MODEL name (for example -FP8 is fp8,
#                -AWQ is awq), else null. See serve_quantization.
#   HF_HUB_OFFLINE=1  skip the Hub revision lookup (revision is then null)
#
# Subcommands: start (default) | stop | print | logs | sut
#   sut  print the SUT JSON to stdout without docker (no GPU needed)

serve_die() { echo "error: $*" >&2; exit 1; }

serve_docker_cmd() {
  local name="$1"
  local cmd=(docker run -d --name "${name}" --gpus "${GPU_DEVICES:-device=0}" --ipc=host
             -p "${PORT:-8000}:8000"
             -v "${HF_HOME:-${HOME}/.cache/huggingface}:/root/.cache/huggingface")
  [[ -n "${HF_TOKEN:-}" ]] && cmd+=(-e HF_TOKEN)
  local e
  # ${a[@]+...} keeps empty arrays safe under set -u on bash 3.2 (macOS).
  for e in ${DOCKER_ENV[@]+"${DOCKER_ENV[@]}"}; do cmd+=(-e "${e}"); done
  cmd+=("${IMAGE}")
  cmd+=(${ENTRYPOINT_CMD[@]+"${ENTRYPOINT_CMD[@]}"})
  cmd+=("${MODEL}" ${SERVE_ARGS[@]+"${SERVE_ARGS[@]}"})
  printf '%q ' "${cmd[@]}"
}

serve_gpu_json() {
  local line=""
  if command -v nvidia-smi >/dev/null 2>&1; then
    line="$(nvidia-smi --query-gpu=name,memory.total,driver_version --format=csv,noheader,nounits \
      -i "${GPU_INDEX:-0}" 2>/dev/null | head -1 || true)"
  fi
  if [[ -z "${line}" ]]; then
    echo "warning: nvidia-smi unavailable or failed; SUT GPU fields marked not captured" >&2
    echo '{"model":"not captured (nvidia-smi unavailable)","count":1,"memory_gb":null,"driver":"not captured"}'
    return 0
  fi
  awk -F', ' '{printf "{\"model\":\"%s\",\"count\":1,\"memory_gb\":%d,\"driver\":\"NVIDIA %s\"}", $1, $2/1024, $3}' <<<"${line}"
}

serve_model_revision() {
  [[ "${HF_HUB_OFFLINE:-}" == 1 ]] && return 0
  curl -fsS --max-time 15 "https://huggingface.co/api/models/${MODEL}/revision/main" 2>/dev/null \
    | jq -r '.sha // empty' 2>/dev/null || true
}

# Quantizer tokens recognized in the MODEL basename, method names before bare
# bit widths so -GPTQ-Int4 is gptq. Token form: <lowercase token>=<SUT value>.
SERVE_QUANT_TOKENS=(nvfp4=nvfp4 mxfp4=mxfp4 fp8=fp8 awq=awq gptq=gptq gguf=gguf
                    w8a8=w8a8 w4a16=w4a16 w4a8=w4a8 bnb=bitsandbytes
                    bitsandbytes=bitsandbytes exl2=exl2 int8=int8 int4=int4 fp4=fp4)

# Prints "<value> <source>", where value is "" for null and source is one of
# env, serve_args, model_name, none.
serve_quantization() {
  if [[ -n "${QUANTIZATION:-}" ]]; then
    if [[ "${QUANTIZATION}" == none ]]; then echo " env"; else echo "${QUANTIZATION} env"; fi
    return 0
  fi
  local i arg next
  local args=(${SERVE_ARGS[@]+"${SERVE_ARGS[@]}"})
  for ((i = 0; i < ${#args[@]}; i++)); do
    arg="${args[i]}"
    case "${arg}" in
      --quantization=*) echo "${arg#*=} serve_args"; return 0 ;;
      --quantization|-q)
        next="${args[i + 1]:-}"
        [[ -n "${next}" ]] && { echo "${next} serve_args"; return 0; }
        ;;
    esac
  done
  # Lowercase, treat . and _ as -, and pad so each token is -delimited.
  local base pair
  base="-$(printf '%s' "${MODEL##*/}" | tr '[:upper:]._' '[:lower:]--')-"
  for pair in "${SERVE_QUANT_TOKENS[@]}"; do
    if [[ "${base}" == *"-${pair%%=*}-"* ]]; then
      echo "${pair#*=} model_name"
      return 0
    fi
  done
  echo " none"
}

# Fails when MODEL overrides DEFAULT_MODEL without SUT_NOTES_OVERRIDE, so a
# SUT never carries notes researched for a different model.
serve_check_model_notes() {
  [[ -z "${DEFAULT_MODEL:-}" || "${MODEL}" == "${DEFAULT_MODEL}" ]] && return 0
  if [[ -z "${SUT_NOTES_OVERRIDE:-}" ]]; then
    serve_die "MODEL=${MODEL} overrides the ${MODALITY} launcher default ${DEFAULT_MODEL}; set SUT_NOTES_OVERRIDE to the researched flags and sources for ${MODEL} (the launcher notes describe ${DEFAULT_MODEL} only)"
  fi
  if [[ -z "${SOURCES_OVERRIDE:-}" ]]; then
    echo "warning: MODEL=${MODEL} without SOURCES_OVERRIDE; extra.launcher_sources still lists the ${DEFAULT_MODEL} sources" >&2
  fi
}

serve_write_sut() {
  # out "-" prints to stdout (plain >/dev/stdout would truncate a redirected file).
  local out="$1" docker_cmd="${2//${HOME}/\~}" gpu rev quant quant_src notes work
  work="$(mktemp)"
  gpu="$(serve_gpu_json)"
  rev="$(serve_model_revision)"
  read -r quant quant_src <<<"$(serve_quantization)" || true
  if [[ -z "${quant_src}" ]]; then quant_src="${quant}"; quant=""; fi
  notes="${SUT_NOTES}"
  if [[ "${quant_src}" == model_name ]]; then
    notes="${notes}; model.quantization=${quant} derived from the MODEL name ${MODEL}"
  fi
  jq -n \
    --arg name "${SUT_NAME:-local GPU / ${MODALITY} live smoke}" \
    --argjson gpu "${gpu}" \
    --arg image "${IMAGE}" \
    --arg config "${docker_cmd}" \
    --arg model "${MODEL}" \
    --arg rev "${rev}" \
    --arg os "$(uname -s) $(uname -r)" \
    --arg notes "${notes}" \
    --arg quant "${quant}" \
    --arg quant_src "${quant_src}" \
    --arg recorded "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --args '{
      provenance: "declared",
      name: $name,
      vendor: "NVIDIA",
      gpu: {model: $gpu.model, count: $gpu.count, memory_gb: $gpu.memory_gb},
      driver_version: $gpu.driver,
      runtime: {name: ($image | split(":")[0] | split("/")[-1]), version: ($image | split(":")[1] // "unknown"), config: $config},
      model: {id: $model, revision: (if $rev == "" then null else $rev end), quantization: (if $quant == "" then null else $quant end)},
      host_os: $os,
      notes: $notes,
      extra: {launcher_sources: ($ARGS.positional | join(" ")), recorded_at_utc: $recorded, image: $image, quantization_source: $quant_src}
    }' "${SOURCES[@]}" >"${work}"
  if [[ "${out}" == - ]]; then cat "${work}"; else cat "${work}" >"${out}"; fi
  rm -f "${work}"
}

serve_main() {
  local sub="${1:-start}"
  # Campaign overrides (space-separated; use --flag=value forms for values
  # that contain spaces). Sources and notes for the override go in the SUT.
  if [[ -n "${SERVE_ARGS_OVERRIDE:-}" ]]; then read -r -a SERVE_ARGS <<<"${SERVE_ARGS_OVERRIDE}"; fi
  if [[ -n "${SOURCES_OVERRIDE:-}" ]]; then read -r -a SOURCES <<<"${SOURCES_OVERRIDE}"; fi
  if [[ -n "${SUT_NOTES_OVERRIDE:-}" ]]; then SUT_NOTES="${SUT_NOTES_OVERRIDE}"; fi
  case "${sub}" in start|sut) serve_check_model_notes ;; esac
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
    sut)
      command -v jq >/dev/null || serve_die "jq is required"
      serve_write_sut - "${docker_cmd}"
      ;;
    start)
      command -v docker >/dev/null || serve_die "docker is required"
      command -v jq >/dev/null || serve_die "jq is required"
      mkdir -p "${out}"
      docker rm -f "${name}" >/dev/null 2>&1 || true
      # SUT first, so a capture failure never leaves a container running.
      serve_write_sut "${out}/sut.json" "${docker_cmd}"
      echo "# ${docker_cmd}" >&2
      eval "${docker_cmd}" >/dev/null
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
    *) serve_die "unknown subcommand: ${sub} (start|stop|print|logs|sut)" ;;
  esac
}
