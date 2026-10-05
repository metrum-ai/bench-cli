#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Metrum AI Bench CLI parity harness: Bench vs AIPerf on the same paced mock,
# same prompts, c=4, 64 measured + 4 warmup, then print the scenario table in
# the epic #184 format.
#
# Usage: scripts/parity/run_pair.sh [OUT_DIR] [SCENARIO ...]
#   SCENARIO: plain | reasoning | slo   (default: all three)
# Env: TOOLS (default "bench aiperf"), BENCH_BIN_DIR, AIPERF,
#      PARITY_INSTALL_AIPERF=1, CONCURRENCY, REQUESTS, WARMUP, MAX_TOKENS, MOCK_PORT, PROMPTS, AIPERF_TOKENIZER, PREFILL_MS,
#      PER_PROMPT_TOKEN_MS, ITL_MS, SLO_TTFT_S, SLO_E2E_S, PRICE_PER_HOUR.
# Exits 3 if a run is not paced (TTFT about E2E). See scripts/parity/README.md.
set -euo pipefail

# shellcheck source=lib.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"
trap parity_cleanup EXIT

OUT="${1:-${PARITY_ROOT}/live-results/parity-$(date -u +%Y%m%dT%H%M%SZ)}"
shift || true
SCENARIOS=("$@")
[[ ${#SCENARIOS[@]} -gt 0 ]] || SCENARIOS=(plain reasoning slo)
SLO_TTFT_S="${SLO_TTFT_S:-0.2}"
SLO_E2E_S="${SLO_E2E_S:-2}"
PRICE_PER_HOUR="${PRICE_PER_HOUR:-2.0}"

mkdir -p "${OUT}"
OUT="$(cd "${OUT}" && pwd)"
COUNTS="${OUT}/counts.jsonl"
# Keep telemetry rows from run_tele.sh in the same OUT; replace client rows.
if [[ -f "${COUNTS}" ]]; then
  grep '"scenario": "telemetry' "${COUNTS}" >"${COUNTS}.keep" || true
  mv "${COUNTS}.keep" "${COUNTS}"
else
  : >"${COUNTS}"
fi

parity_prompts "${OUT}"
if parity_want bench; then
  BENCH="$(parity_bench_bin metrum-ai-bench-cli-llm)"
  parity_log "bench: $(parity_bench_identity "${BENCH}")"
  parity_sut "${OUT}" "${BENCH}"
fi
if parity_want aiperf; then
  AIPERF_BIN="$(parity_aiperf "${OUT}")"
  parity_log "aiperf: ${AIPERF_BIN} $("${AIPERF_BIN}" --version 2>/dev/null | tail -n1)"
fi
URL="http://127.0.0.1:${MOCK_PORT}"

for sc in "${SCENARIOS[@]}"; do
  dir="${OUT}/${sc}"
  mkdir -p "${dir}"
  mock_args=()
  bench_extra=()
  aiperf_extra=()
  case "${sc}" in
    plain) ;;
    reasoning) mock_args=(--reasoning) ;;
    slo)
      bench_extra=(--slo "ttft=${SLO_TTFT_S}" --slo "e2e=${SLO_E2E_S}" --price-per-hour "${PRICE_PER_HOUR}")
      # AIPerf goodput takes display units (ms). AIPerf has no cost metric.
      to_ms='import sys; print(f"{float(sys.argv[1]) * 1000:g}")'
      ttft_ms="$("${PYTHON}" -c "${to_ms}" "${SLO_TTFT_S}")"
      e2e_ms="$("${PYTHON}" -c "${to_ms}" "${SLO_E2E_S}")"
      aiperf_extra=(--goodput "time_to_first_token:${ttft_ms} request_latency:${e2e_ms}")
      ;;
    *) echo "error: unknown scenario ${sc} (plain|reasoning|slo)" >&2; exit 2 ;;
  esac

  # Fresh mock per tool so /metrics counters start at zero for each.
  if parity_want bench; then
    parity_log "${sc}: bench"
    parity_start_mock "${dir}/mock-bench.log" "${mock_args[@]}"
    "${BENCH}" --quiet --scenario "parity-${sc}" \
      --url "${URL}/v1/chat/completions" --api-key dummy --model "${MODEL}" \
      --mode chat --streaming --prompts "${OUT}/prompts.jsonl" --max-tokens "${MAX_TOKENS}" \
      --num-requests "$((REQUESTS + WARMUP))" --warmup-requests "${WARMUP}" \
      --concurrency "${CONCURRENCY}" --sut "${OUT}/sut.json" --require-sut \
      --data-log "${dir}/bench.jsonl" --debug-log "${dir}/bench-debug.log" \
      --error-log "${dir}/bench-error.log" "${bench_extra[@]}" >"${dir}/bench-stdout.txt" 2>&1
    parity_stop_last
    "${PYTHON}" "${PARITY_DIR}/count_points.py" bench "${dir}/bench.jsonl" \
      --scenario "${sc}" --append "${COUNTS}" >"${dir}/count-bench.json"
  fi

  if parity_want aiperf; then
    parity_log "${sc}: aiperf"
    parity_start_mock "${dir}/mock-aiperf.log" "${mock_args[@]}"
    "${AIPERF_BIN}" profile --model "${MODEL}" --tokenizer "${AIPERF_TOKENIZER}" \
      --url "${URL}" --endpoint-type chat --streaming \
      --concurrency "${CONCURRENCY}" --request-count "${REQUESTS}" \
      --warmup-request-count "${WARMUP}" \
      --input-file "${OUT}/aiperf-input.jsonl" --custom-dataset-type single_turn \
      --extra-inputs "max_tokens:${MAX_TOKENS}" --no-gpu-telemetry --ui none \
      --artifact-dir "${dir}/aiperf" "${aiperf_extra[@]}" \
      >"${dir}/aiperf-stdout.txt" 2>"${dir}/aiperf-stderr.txt"
    parity_stop_last
    "${PYTHON}" "${PARITY_DIR}/count_points.py" aiperf "${dir}/aiperf" \
      --scenario "${sc}" --append "${COUNTS}" >"${dir}/count-aiperf.json"
  fi
done

"${PYTHON}" "${PARITY_DIR}/count_points.py" table "${COUNTS}" | tee "${OUT}/table.md"
parity_log "results under ${OUT}"
