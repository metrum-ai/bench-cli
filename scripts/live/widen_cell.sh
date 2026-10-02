#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Metrum AI Bench CLI: run one widen-smoke cell ON the GPU host, wrapped in
# telemetry sidecars (all-smi fork plus the vLLM /metrics endpoint), then
# gate it with assert_headline.sh.
#
# Usage: widen_cell.sh OUT_DIR MODALITY BIN [bench args...]
#   OUT_DIR   cell directory (results.jsonl, telemetry.ndjson, command.txt, ...)
#   MODALITY  llm|vlm|asr|imagegen|strategic (strategic skips assert_headline)
#   BIN       bench binary path
# Environment: ALLSMI_URL (default http://127.0.0.1:9090/metrics),
#   VLLM_METRICS_URL (default http://127.0.0.1:8000/metrics)
set -uo pipefail

out="$1" modality="$2" bin="$3"
shift 3
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mkdir -p "${out}"

python3 "${here}/telemetry_sidecar.py" "${out}/telemetry.ndjson" --src all-smi \
  --url "${ALLSMI_URL:-http://127.0.0.1:9090/metrics}" --interval-ms 500 \
  --include '^all_smi_(gpu|cpu|memory)_' &
side1=$!
python3 "${here}/telemetry_sidecar.py" "${out}/telemetry.ndjson" --src vllm \
  --url "${VLLM_METRICS_URL:-http://127.0.0.1:8000/metrics}" --interval-ms 1000 \
  --include '^vllm:(gpu_cache_usage_perc|kv_cache_usage_perc|num_requests_(running|waiting)|num_preemptions_total|generation_tokens_total|prompt_tokens_total)$' &
side2=$!

printf '%q ' "${bin}" "$@" >"${out}/command.txt"; echo >>"${out}/command.txt"
date -u +%Y-%m-%dT%H:%M:%SZ >"${out}/started_at_utc.txt"
"${bin}" "$@" >"${out}/stdout.txt" 2>&1
rc=$?
echo "${rc}" >"${out}/exit_code.txt"
date -u +%Y-%m-%dT%H:%M:%SZ >"${out}/finished_at_utc.txt"
kill "${side1}" "${side2}" 2>/dev/null; wait "${side1}" "${side2}" 2>/dev/null

if [[ "${modality}" != strategic && -f "${out}/results.jsonl" ]]; then
  args=()
  [[ "${modality}" == imagegen ]] && args=(--artifact-dir "${out}/artifacts")
  "${here}/assert_headline.sh" "${modality}" "${out}/results.jsonl" ${args[@]+"${args[@]}"} >"${out}/assert.txt" 2>&1
  echo "assert_rc=$?" >>"${out}/assert.txt"
fi
echo "cell ${out##*/} rc=${rc} $(tail -n 2 "${out}/assert.txt" 2>/dev/null | head -1)"
tail -n 12 "${out}/stdout.txt"
