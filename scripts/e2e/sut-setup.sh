#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Idempotent SUT setup on a Shadeform GPU box for telemetry e2e.
# Prefer Qwen/Qwen3.8-27B; scrape all-smi from the Metrum fork on /metric.
set -euo pipefail

MODEL="${MODEL:-Qwen/Qwen3.8-27B}"
SERVED_NAME="${SERVED_NAME:-sut}"
VLLM_IMAGE="${VLLM_IMAGE:-vllm/vllm-openai:latest}"
MAX_MODEL_LEN="${MAX_MODEL_LEN:-16384}"
GPU_MEM_UTIL="${GPU_MEM_UTIL:-0.90}"
# Qwen3.5/3.8 hybrid (Mamba) needs max_num_seqs <= available mamba blocks.
MAX_NUM_SEQS="${MAX_NUM_SEQS:-128}"
# Shadeform/Massed Compute VMs often reject CUDA graph capture
# (torch.AcceleratorError: operation not permitted). Prefer eager.
VLLM_ENFORCE_EAGER="${VLLM_ENFORCE_EAGER:-1}"
HF_HOME="${HF_HOME:-$HOME/.cache/huggingface}"
ALL_SMI_REPO="${ALL_SMI_REPO:-https://github.com/chetan-metrum-ai/all-smi}"
ALL_SMI_RELEASE="${ALL_SMI_RELEASE:-v0.26.3-metrum.3}"
ALL_SMI_BIN_DIR="${ALL_SMI_BIN_DIR:-${HOME}/.local/bin}"

log() { echo "[sut-setup $(date -u +%Y-%m-%dT%H:%M:%SZ)] $*"; }

need() { command -v "$1" >/dev/null 2>&1 || { echo "missing $1" >&2; exit 1; }; }

install_all_smi_release() {
  local arch asset url tmp
  arch="$(uname -m)"
  case "${arch}" in
    x86_64|amd64) asset="all-smi-linux-x86_64.tar.gz" ;;
    aarch64|arm64) asset="all-smi-linux-aarch64.tar.gz" ;;
    *)
      echo "unsupported arch for all-smi release: ${arch}" >&2
      return 1
      ;;
  esac
  url="${ALL_SMI_REPO}/releases/download/${ALL_SMI_RELEASE}/${asset}"
  tmp="$(mktemp -d)"
  log "downloading all-smi ${ALL_SMI_RELEASE} (${asset})"
  curl -fsSL -o "${tmp}/${asset}" "${url}"
  tar -xzf "${tmp}/${asset}" -C "${tmp}"
  mkdir -p "${ALL_SMI_BIN_DIR}"
  install -m 0755 "${tmp}/all-smi" "${ALL_SMI_BIN_DIR}/all-smi"
  # Companion libs ship beside the binary in the release archive.
  for so in "${tmp}"/liball_smi_*.so; do
    [[ -f "${so}" ]] || continue
    install -m 0644 "${so}" "${ALL_SMI_BIN_DIR}/$(basename "${so}")"
  done
  rm -rf "${tmp}"
  export PATH="${ALL_SMI_BIN_DIR}:${PATH}"
  # Prefer loading sibling .so files when present.
  export LD_LIBRARY_PATH="${ALL_SMI_BIN_DIR}:${LD_LIBRARY_PATH:-}"
  log "installed $(command -v all-smi) ($(all-smi --version 2>/dev/null || echo ok))"
}

need curl
need docker
need jq

mkdir -p "$HF_HOME" /tmp/metrum-e2e
export PATH="${ALL_SMI_BIN_DIR}:${HOME}/.cargo/bin:${PATH}"

# --- all-smi (Metrum fork) on 127.0.0.1:9090/metric ---
if ! curl -fsS --max-time 2 http://127.0.0.1:9090/metric >/dev/null 2>&1 \
  && ! curl -fsS --max-time 2 http://127.0.0.1:9090/metrics >/dev/null 2>&1; then
  if ! command -v all-smi >/dev/null 2>&1; then
    install_all_smi_release
  fi
  need all-smi
  export LD_LIBRARY_PATH="${ALL_SMI_BIN_DIR}:${LD_LIBRARY_PATH:-}"
  nohup env LD_LIBRARY_PATH="${ALL_SMI_BIN_DIR}:${LD_LIBRARY_PATH:-}" \
    all-smi api --port 9090 >/tmp/metrum-e2e/all-smi.log 2>&1 &
  echo $! >/tmp/metrum-e2e/all-smi.pid
  for _ in $(seq 1 30); do
    if curl -fsS --max-time 2 http://127.0.0.1:9090/metric >/dev/null 2>&1 \
      || curl -fsS --max-time 2 http://127.0.0.1:9090/metrics >/dev/null 2>&1; then
      break
    fi
    sleep 1
  done
fi
ALL_SMI_PATH="/metric"
if ! curl -fsS --max-time 2 "http://127.0.0.1:9090${ALL_SMI_PATH}" >/dev/null 2>&1; then
  ALL_SMI_PATH="/metrics"
fi
log "all-smi ok at http://127.0.0.1:9090${ALL_SMI_PATH}"

# --- node_exporter on 127.0.0.1:9100 ---
if ! curl -fsS --max-time 2 http://127.0.0.1:9100/metrics >/dev/null 2>&1; then
  log "starting node_exporter"
  docker rm -f metrum-node-exporter >/dev/null 2>&1 || true
  docker run -d --name metrum-node-exporter --net=host \
    --pid=host \
    -v /:/host:ro,rslave \
    quay.io/prometheus/node-exporter:latest \
    --path.rootfs=/host \
    --web.listen-address=127.0.0.1:9100 \
    --collector.rapl \
    --collector.hwmon
fi

# --- dcgm-exporter on 127.0.0.1:9400 (best effort) ---
if ! curl -fsS --max-time 2 http://127.0.0.1:9400/metrics >/dev/null 2>&1; then
  log "starting dcgm-exporter (best effort)"
  docker rm -f metrum-dcgm >/dev/null 2>&1 || true
  docker run -d --name metrum-dcgm --gpus all --cap-add SYS_ADMIN --net=host \
    -e DCGM_EXPORTER_LISTEN=:9400 \
    nvcr.io/nvidia/k8s/dcgm-exporter:3.3.8-3.6.0-ubuntu22.04 \
    -a 127.0.0.1 || log "dcgm-exporter failed; continuing with all-smi only"
fi

# --- cAdvisor on 127.0.0.1:8080 (best effort) ---
if ! curl -fsS --max-time 2 http://127.0.0.1:8080/metrics >/dev/null 2>&1; then
  log "starting cadvisor"
  docker rm -f metrum-cadvisor >/dev/null 2>&1 || true
  docker run -d --name metrum-cadvisor --net=host \
    --volume=/:/rootfs:ro \
    --volume=/var/run:/var/run:ro \
    --volume=/sys:/sys:ro \
    --volume=/var/lib/docker/:/var/lib/docker:ro \
    --publish=127.0.0.1:8080:8080 \
    gcr.io/cadvisor/cadvisor:v0.49.1 \
    --port=8080 || log "cadvisor failed; continuing"
fi

# --- vLLM ---
if ! curl -fsS --max-time 2 http://127.0.0.1:8000/v1/models >/dev/null 2>&1; then
  log "starting vLLM ${MODEL}"
  docker rm -f metrum-vllm >/dev/null 2>&1 || true
  vllm_args=(
    "${MODEL}"
    --host 127.0.0.1
    --port 8000
    --served-model-name "${SERVED_NAME}"
    --max-model-len "${MAX_MODEL_LEN}"
    --gpu-memory-utilization "${GPU_MEM_UTIL}"
    --max-num-seqs "${MAX_NUM_SEQS}"
    --dtype auto
  )
  if [[ "${VLLM_ENFORCE_EAGER}" == "1" ]]; then
    vllm_args+=(--enforce-eager)
  fi
  docker run -d --name metrum-vllm --gpus all --ipc=host --net=host \
    -e HF_TOKEN="${HF_TOKEN:-}" \
    -e HUGGING_FACE_HUB_TOKEN="${HF_TOKEN:-}" \
    -v "${HF_HOME}:/root/.cache/huggingface" \
    "${VLLM_IMAGE}" \
    "${vllm_args[@]}"
  log "waiting for vLLM (up to 20 min; max_num_seqs=${MAX_NUM_SEQS} enforce_eager=${VLLM_ENFORCE_EAGER})"
  ready=0
  for i in $(seq 1 240); do
    if curl -fsS --max-time 3 http://127.0.0.1:8000/v1/models >/dev/null 2>&1; then
      ready=1
      break
    fi
    # If the container exited, dump logs and fail fast instead of waiting out the timer.
    if ! docker ps --format '{{.Names}}' | grep -qx metrum-vllm; then
      log "vLLM container exited early"
      docker logs metrum-vllm 2>&1 | tail -120 || true
      exit 1
    fi
    sleep 5
    if (( i % 12 == 0 )); then log "still waiting for vLLM (${i}/240)"; fi
  done
  if [[ "${ready}" -ne 1 ]]; then
    log "vLLM failed to become ready"
    docker logs metrum-vllm 2>&1 | tail -120 || true
    exit 1
  fi
fi
log "vLLM ok"

# Confirm engine metrics
if ! curl -fsS http://127.0.0.1:8000/metrics | grep -E 'vllm:(gpu_cache_usage_perc|num_requests_running)' >/dev/null; then
  log "warning: expected vllm metrics not found on /metrics"
fi

# Write telemetry.yaml with discovered all-smi path
cat > /tmp/metrum-e2e/telemetry.yaml <<EOF
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
default_interval_ms: 1000
timeout_ms: 800
sources:
  - name: all-smi
    url: http://127.0.0.1:9090${ALL_SMI_PATH}
    interval_ms: 500
    include:
      - "^all_smi_(gpu|cpu|memory)_"
  - name: vllm
    url: http://127.0.0.1:8000/metrics
    interval_ms: 1000
    include:
      - "^vllm:(gpu_cache_usage_perc|kv_cache_usage_perc|num_requests_(running|waiting)|num_preemptions_total|generation_tokens_total)\$"
  - name: node
    url: http://127.0.0.1:9100/metrics
    interval_ms: 1000
    include:
      - "^node_(cpu_seconds_total|memory_MemAvailable_bytes|rapl_.*_joules_total|network_(receive|transmit)_bytes_total)\$"
    allow_empty: true
EOF

if curl -fsS --max-time 2 http://127.0.0.1:9400/metrics >/dev/null 2>&1; then
  cat >> /tmp/metrum-e2e/telemetry.yaml <<'EOF'
  - name: dcgm
    url: http://127.0.0.1:9400/metrics
    interval_ms: 250
    include:
      - "^DCGM_FI_(DEV_(POWER_USAGE|TOTAL_ENERGY_CONSUMPTION|GPU_UTIL|FB_USED|SM_CLOCK|GPU_TEMP)|PROF_(SM_ACTIVE|PIPE_TENSOR_ACTIVE|DRAM_ACTIVE))$"
    units:
      DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION: {scale: 0.001, unit: J}
    allow_empty: true
EOF
fi

if curl -fsS --max-time 2 http://127.0.0.1:8080/metrics >/dev/null 2>&1; then
  cat >> /tmp/metrum-e2e/telemetry.yaml <<'EOF'
  - name: cadvisor
    url: http://127.0.0.1:8080/metrics
    interval_ms: 1000
    include:
      - "^container_(cpu_usage_seconds_total|memory_working_set_bytes)$"
    allow_empty: true
EOF
fi

REV="$(curl -fsS -H "Authorization: Bearer ${HF_TOKEN:-}" \
  "https://huggingface.co/api/models/${MODEL}" 2>/dev/null | jq -r '.sha // empty' || true)"

GPU_NAME="$(nvidia-smi --query-gpu=name --format=csv,noheader | head -1 | tr -d '"')"
DRIVER_VER="$(nvidia-smi --query-gpu=driver_version --format=csv,noheader | head -1)"
HOST_OS="$(uname -srm)"
# runtime.config must be a string (SutRuntime), not a JSON object.
LAUNCH="docker run --gpus all -p 8000:8000 ${VLLM_IMAGE} --model ${MODEL}"
LAUNCH+=" --served-model-name ${SERVED_NAME} --dtype auto --max-model-len ${MAX_MODEL_LEN}"
LAUNCH+=" --gpu-memory-utilization ${GPU_MEM_UTIL} --max-num-seqs ${MAX_NUM_SEQS}"
if [[ "${VLLM_ENFORCE_EAGER}" == "1" ]]; then
  LAUNCH+=" --enforce-eager"
fi
LAUNCH+=" --host 0.0.0.0 --port 8000"
export SUT_GPU_NAME="${GPU_NAME}" SUT_DRIVER_VER="${DRIVER_VER}" SUT_HOST_OS="${HOST_OS}"
export SUT_LAUNCH="${LAUNCH}" SUT_MODEL="${MODEL}" SUT_REV="${REV}"
export SUT_VLLM_IMAGE="${VLLM_IMAGE}" SUT_ALL_SMI_REPO="${ALL_SMI_REPO}"
export SUT_ALL_SMI_PATH="${ALL_SMI_PATH}" SUT_ENFORCE_EAGER="${VLLM_ENFORCE_EAGER}"
python3 - <<'PY'
import json, os
rev = os.environ.get("SUT_REV") or None
sut = {
  "provenance": "declared",
  "name": f"shadeform-e2e / {os.environ['SUT_GPU_NAME']} x1",
  "gpu": {"model": os.environ["SUT_GPU_NAME"], "count": 1},
  "driver_version": os.environ["SUT_DRIVER_VER"],
  "runtime": {
    "name": "vllm",
    "version": os.environ["SUT_VLLM_IMAGE"],
    "config": os.environ["SUT_LAUNCH"],
  },
  "model": {"id": os.environ["SUT_MODEL"], "revision": rev, "quantization": None},
  "host_os": os.environ["SUT_HOST_OS"],
  "notes": (
    f"Shadeform e2e; all-smi from {os.environ['SUT_ALL_SMI_REPO']} "
    f"path {os.environ['SUT_ALL_SMI_PATH']}; enforce_eager={os.environ['SUT_ENFORCE_EAGER']}"
  ),
}
with open("/tmp/metrum-e2e/sut.json", "w", encoding="utf-8") as f:
  json.dump(sut, f, indent=2)
  f.write("\n")
PY

log "wrote /tmp/metrum-e2e/sut.json and /tmp/metrum-e2e/telemetry.yaml"
log "done"
