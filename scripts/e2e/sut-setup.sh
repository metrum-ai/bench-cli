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
HF_HOME="${HF_HOME:-$HOME/.cache/huggingface}"
ALL_SMI_REPO="${ALL_SMI_REPO:-https://github.com/chetan-metrum-ai/all-smi}"

log() { echo "[sut-setup $(date -u +%Y-%m-%dT%H:%M:%SZ)] $*"; }

need() { command -v "$1" >/dev/null 2>&1 || { echo "missing $1" >&2; exit 1; }; }

need curl
need docker
need jq

mkdir -p "$HF_HOME" /tmp/metrum-e2e

# --- all-smi (Metrum fork) on 127.0.0.1:9090/metric ---
if ! curl -fsS --max-time 2 http://127.0.0.1:9090/metric >/dev/null 2>&1 \
  && ! curl -fsS --max-time 2 http://127.0.0.1:9090/metrics >/dev/null 2>&1; then
  log "installing all-smi from ${ALL_SMI_REPO}"
  if ! command -v all-smi >/dev/null 2>&1; then
    need cargo
    cargo install --git "${ALL_SMI_REPO}" --locked all-smi || \
      cargo install --git "${ALL_SMI_REPO}" all-smi
  fi
  nohup all-smi api --port 9090 >/tmp/metrum-e2e/all-smi.log 2>&1 &
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
  docker run -d --name metrum-vllm --gpus all --ipc=host --net=host \
    -e HF_TOKEN="${HF_TOKEN:-}" \
    -e HUGGING_FACE_HUB_TOKEN="${HF_TOKEN:-}" \
    -v "${HF_HOME}:/root/.cache/huggingface" \
    "${VLLM_IMAGE}" \
    --model "${MODEL}" \
    --host 127.0.0.1 \
    --port 8000 \
    --served-model-name "${SERVED_NAME}" \
    --max-model-len "${MAX_MODEL_LEN}" \
    --gpu-memory-utilization "${GPU_MEM_UTIL}" \
    --dtype auto
  log "waiting for vLLM (up to 20 min)"
  ready=0
  for i in $(seq 1 240); do
    if curl -fsS --max-time 3 http://127.0.0.1:8000/v1/models >/dev/null 2>&1; then
      ready=1
      break
    fi
    sleep 5
    if (( i % 12 == 0 )); then log "still waiting for vLLM (${i}/240)"; fi
  done
  if [[ "${ready}" -ne 1 ]]; then
    log "vLLM failed to become ready"
    docker logs metrum-vllm 2>&1 | tail -80 || true
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

cat > /tmp/metrum-e2e/sut.json <<EOF
{
  "gpu": {"model": "$(nvidia-smi --query-gpu=name --format=csv,noheader | head -1 | tr -d '"')", "count": 1},
  "driver_version": "$(nvidia-smi --query-gpu=driver_version --format=csv,noheader | head -1)",
  "runtime": {
    "name": "vllm",
    "version": "${VLLM_IMAGE}",
    "config": {
      "model": "${MODEL}",
      "revision": "${REV}",
      "dtype": "auto",
      "max_model_len": ${MAX_MODEL_LEN},
      "gpu_memory_utilization": ${GPU_MEM_UTIL},
      "served_model_name": "${SERVED_NAME}",
      "host": "127.0.0.1",
      "port": 8000
    }
  },
  "host_os": "$(uname -srm)",
  "notes": "Shadeform e2e; all-smi from ${ALL_SMI_REPO} path ${ALL_SMI_PATH}"
}
EOF

log "wrote /tmp/metrum-e2e/sut.json and /tmp/metrum-e2e/telemetry.yaml"
log "done"
