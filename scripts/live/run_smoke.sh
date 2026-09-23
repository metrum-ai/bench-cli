#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Orchestrate wait_for_vllm + metrum-ai-bench-cli-llm against an already-running
# OpenAI-compatible endpoint (vLLM / SGLang). Does NOT create Shadeform
# instances — point --host at a live IP (or localhost).
#
# Requires: wait_for_vllm and metrum-ai-bench-cli-llm on PATH (or built under
# target/{debug,release}/). Results land in live-results/ (gitignored).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RESULTS_DIR="${RESULTS_DIR:-${REPO_ROOT}/live-results}"
LLM_MODEL="${LLM_MODEL:-Qwen/Qwen2.5-7B-Instruct}"

usage() {
  cat <<'EOF'
Usage: run_smoke.sh --host HOST [--port 8000]

Requires a running instance (Shadeform or local docker). Modest load:
  wait_for_vllm, then metrum-ai-bench-cli-llm with 4 requests / concurrency 1.

Does not create or delete Shadeform VMs. If you created a GPU with
shadeform.sh create --execute, keep a trap DELETE around your session:

  INSTANCE_ID=...
  cleanup() { ./scripts/live/shadeform.sh delete "$INSTANCE_ID" || true; }
  trap cleanup EXIT
EOF
}

die() { echo "error: $*" >&2; exit 1; }

host=""
port="8000"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --host) host="$2"; shift 2 ;;
    --port) port="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done
[[ -n "${host}" ]] || { usage; exit 1; }

resolve_wait() {
  if command -v wait_for_vllm >/dev/null 2>&1; then
    echo wait_for_vllm
    return
  fi
  local c
  for c in "${REPO_ROOT}/target/release/wait_for_vllm" "${REPO_ROOT}/target/debug/wait_for_vllm"; do
    if [[ -x "${c}" ]]; then
      echo "${c}"
      return
    fi
  done
  die "binary not found: wait_for_vllm"
}

WAIT_BIN="$(resolve_wait)"

echo "# waiting for vLLM/SGLang at ${host}:${port}" >&2
"${WAIT_BIN}" \
  --host "${host}" \
  --port "${port}" \
  --max-retries 30 \
  --retry-delay-seconds 5 \
  --extra-pause-seconds 0

url="http://${host}:${port}/v1/chat/completions"
echo "# smoke LLM against ${url}" >&2
"${SCRIPT_DIR}/shadeform.sh" run-llm \
  --url "${url}" \
  --num-requests 4 \
  --concurrency 1 \
  --model "${LLM_MODEL}" \
  --out-dir "${RESULTS_DIR}/smoke-$(date -u +%Y%m%d-%H%M%S)"
