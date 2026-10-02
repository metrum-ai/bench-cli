#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Wait for an already-running OpenAI-compatible endpoint (vLLM / SGLang), then
# run metrum-ai-bench-cli-llm against it. Does NOT create Shadeform instances;
# point --host at a live IP (or localhost).
#
# --local --modality {llm,vlm,asr,imagegen} hands off to local_smoke.sh, which
# runs one cell against a server on this host and gates it with
# assert_headline.sh.
#
# Requires curl and metrum-ai-bench-cli-llm on PATH (or built under
# target/{debug,release}/). Results land in live-results/ (gitignored).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RESULTS_DIR="${RESULTS_DIR:-${REPO_ROOT}/live-results}"
LLM_MODEL="${LLM_MODEL:-Qwen/Qwen2.5-7B-Instruct}"

usage() {
  cat <<'EOF'
Usage: run_smoke.sh --host HOST [--port 8000]
       run_smoke.sh --local --modality {llm,vlm,asr,imagegen} [local_smoke.sh options]

Requires a running instance (Shadeform or local docker). Modest load:
  wait for /v1/models, then metrum-ai-bench-cli-llm with 4 requests /
  concurrency 1 on a Hub prompt mix (scripts/live/lib/hub_prompts.sh).

Does not create or delete Shadeform VMs. If you created a GPU with
shadeform.sh create --execute, keep a trap DELETE around your session:

  INSTANCE_ID=...
  cleanup() { ./scripts/live/shadeform.sh delete "$INSTANCE_ID" || true; }
  trap cleanup EXIT
EOF
}

die() { echo "error: $*" >&2; exit 1; }

if [[ "${1:-}" == --local ]]; then
  exec "${SCRIPT_DIR}/local_smoke.sh" "$@"
fi

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

# wait_for_vllm was never a shipped binary; poll /v1/models instead.
echo "# waiting for vLLM/SGLang at ${host}:${port}" >&2
for ((i = 1; i <= 30; i++)); do
  curl -fsS -o /dev/null --max-time 5 "http://${host}:${port}/v1/models" && break
  [[ "${i}" -lt 30 ]] || die "endpoint ${host}:${port} not ready after 30 tries"
  sleep 5
done

url="http://${host}:${port}/v1/chat/completions"
echo "# smoke LLM against ${url}" >&2
"${SCRIPT_DIR}/shadeform.sh" run-llm \
  --url "${url}" \
  --num-requests 4 \
  --concurrency 1 \
  --model "${LLM_MODEL}" \
  --out-dir "${RESULTS_DIR}/smoke-$(date -u +%Y%m%d-%H%M%S)"
