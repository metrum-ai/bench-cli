#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Metrum AI Bench CLI: one live smoke cell against a server on this host (no
# Shadeform provisioning), followed by cargo xtask assert-headline.
#
# Usage:
#   local_smoke.sh --local --modality {llm,vlm,asr,imagegen}
#                  [--url http://127.0.0.1:8000] [--model ID] [--sut FILE]
#                  [--out DIR] [--num-requests N] [--concurrency C]
#
# Start the server first with scripts/live/serve/<modality>.sh, which also
# writes the SUT used here by default (live-results/serve-<modality>/sut.json).
#
# Inputs per modality:
#   llm       prompts extracted from the Hub with metrum-ai-bench-cli-prompts
#             (scripts/live/lib/hub_prompts.sh defaults: metrum-ai/prompt-library,
#             config sample, profile chat-short). Dataset, resolved revision
#             SHA, profile, and row count are stamped into the SUT notes.
#             Thinking is disabled per request (LLM_EXTRA_BODY) so the
#             chat-short cap reaches visible text.
#   vlm       test-data/vlm/prompts.jsonl (512x512 PNG)
#   asr       test-data/asr/ LibriSpeech clips with --ground-truth (WER/CER)
#   imagegen  1024x1024, 9 steps, guidance 0.0 (Z-Image-Turbo model card)
# VLM, ASR, and imagegen have no Hub prompt dataset; local fixtures are used.
#
# Exit status is cargo xtask assert-headline's: nonzero when the cell cannot back a
# headline claim. Results land in live-results/ (gitignored).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
# shellcheck source=scripts/live/lib/hub_prompts.sh
source "${SCRIPT_DIR}/lib/hub_prompts.sh"
# shellcheck source=scripts/live/lib/bench_bin.sh
source "${SCRIPT_DIR}/lib/bench_bin.sh"

die() { echo "error: $*" >&2; exit 1; }
usage() { sed -n '5,28p' "$0" >&2; exit 2; }

local_mode=0 modality="" base="http://127.0.0.1:${PORT:-8000}" model="" sut="" out=""
num_requests="" concurrency="${CONCURRENCY:-2}" warmup="${WARMUP:-1}" seed="${SEED:-7}"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --local) local_mode=1; shift ;;
    --modality) modality="$2"; shift 2 ;;
    --url) base="${2%/}"; shift 2 ;;
    --model) model="$2"; shift 2 ;;
    --sut) sut="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    --num-requests) num_requests="$2"; shift 2 ;;
    --concurrency) concurrency="$2"; shift 2 ;;
    -h|--help) usage ;;
    *) die "unknown option: $1" ;;
  esac
done
[[ "${local_mode}" -eq 1 ]] || die "only --local is supported here; use matrix_smoke.sh or campaign.sh for Shadeform"
case "${modality}" in
  llm) model="${model:-Qwen/Qwen3-8B}"; num_requests="${num_requests:-16}" ;;
  vlm) model="${model:-Qwen/Qwen3-VL-8B-Instruct}"; num_requests="${num_requests:-8}" ;;
  asr) model="${model:-openai/whisper-large-v3-turbo}"; num_requests="${num_requests:-6}" ;;
  imagegen) model="${model:-Tongyi-MAI/Z-Image-Turbo}"; num_requests="${num_requests:-4}" ;;
  *) usage ;;
esac
command -v jq >/dev/null || die "jq is required"
sut="${sut:-${REPO_ROOT}/live-results/serve-${modality}/sut.json}"
[[ -f "${sut}" ]] || die "SUT file ${sut} not found; start scripts/live/serve/${modality}.sh or pass --sut"
out="${out:-${REPO_ROOT}/live-results/local-${modality}-$(date -u +%Y%m%d-%H%M%S)}"
mkdir -p "${out}"
cp "${sut}" "${out}/sut.json"

resolve_bin() {
  # Prebuilt binaries only (scripts/live/lib/bench_bin.sh); never compiles.
  bench_bin_resolve "${REPO_ROOT}" "$1" || die "binary not found: $1"
}

deadline=$((SECONDS + ${READY_TIMEOUT_S:-600}))
until curl -fsS -o /dev/null --max-time 5 "${base}/v1/models"; do
  (( SECONDS < deadline )) || die "timed out waiting for ${base}/v1/models"
  sleep 5
done

total=$((num_requests + warmup))
common=(--api-key dummy --scenario "local-smoke-${modality}" --model "${model}"
        --num-requests "${total}" --warmup-requests "${warmup}" --concurrency "${concurrency}"
        --data-log "${out}/results.jsonl" --debug-log "${out}/debug.log"
        --error-log "${out}/error.log" --sut "${out}/sut.json" --require-sut)
assert_args=()
case "${modality}" in
  llm)
    hub_prompts_extract "${REPO_ROOT}" "${out}/prompts.jsonl" "${out}/prompt-mix.json" "${total}"
    hub_prompts_stamp_sut "${out}/sut.json" "${out}/prompt-mix.json"
    # --count is a soft target; size the run to the rows actually selected
    # so every selected prompt is sent once (docs/LIMITATIONS.md).
    total="$(jq -r '.selected_count' "${out}/prompt-mix.json")"
    for i in "${!common[@]}"; do
      [[ "${common[$i]}" == --num-requests ]] && common[i+1]="${total}"
    done
    max_tokens="${MAX_TOKENS:-$(jq -r '.recommended_max_tokens' "${out}/prompt-mix.json")}"
    no_thinking='{"chat_template_kwargs":{"enable_thinking":false}}'
    cmd=("$(resolve_bin metrum-ai-bench-cli-llm)" --url "${base}/v1/chat/completions" --mode chat --streaming
         --prompts "${out}/prompts.jsonl" --max-tokens "${max_tokens}" --log-level warn
         --extra-body-json "${LLM_EXTRA_BODY:-${no_thinking}}")
    ;;
  vlm)
    jq -c --arg root "${REPO_ROOT}" '.image_url = ($root + "/" + .image_url)' \
      "${REPO_ROOT}/test-data/vlm/prompts.jsonl" >"${out}/prompts.jsonl"
    cmd=("$(resolve_bin metrum-ai-bench-cli-vlm)" --url "${base}/v1/chat/completions" --streaming
         --prompts "${out}/prompts.jsonl" --max-tokens "${MAX_TOKENS:-64}" --log-level warn)
    ;;
  asr)
    jq -c --arg root "${REPO_ROOT}" '.path = ($root + "/" + .path)' \
      "${REPO_ROOT}/test-data/asr/input.jsonl" >"${out}/input.jsonl"
    cmd=("$(resolve_bin metrum-ai-bench-cli-asr)" --url "${base}/v1/audio/transcriptions"
         --input "${out}/input.jsonl" --ground-truth "${REPO_ROOT}/test-data/asr/truth.jsonl"
         --response-format json --log-level warn)
    ;;
  imagegen)
    cmd=("$(resolve_bin metrum-ai-bench-cli-imagegen)" --url "${base}/v1"
         --prompt "${IMAGEGEN_PROMPT:-A red circle, a blue square, and a green triangle on a white background}"
         --size "${IMAGEGEN_SIZE:-1024x1024}" --num-inference-steps "${IMAGEGEN_STEPS:-9}"
         --guidance-scale "${IMAGEGEN_GUIDANCE:-0.0}" --seed "${seed}" --artifact-dir "${out}/artifacts")
    assert_args=(--artifact-dir "${out}/artifacts")
    ;;
esac
[[ "${modality}" == imagegen ]] || cmd+=(--seed "${seed}")
# Record which binary ran (path, --version, checkout) in the SUT, so a tip
# build that still prints the last release version is not taken for it.
ident="$(bench_bin_identity "${cmd[0]}" "${REPO_ROOT}")"
jq --arg id "${ident}" '.notes = ((.notes // "") + (if (.notes // "") == "" then "" else "; " end) + "bench binary: " + $id)
  | .extra = ((.extra // {}) + {bench_binary: $id})' "${out}/sut.json" >"${out}/sut.tmp" && mv "${out}/sut.tmp" "${out}/sut.json"
cmd+=("${common[@]}")

printf '%q ' "${cmd[@]}" >"${out}/command.txt"; echo >>"${out}/command.txt"
echo "# ${modality}: $(cat "${out}/command.txt")" >&2
set +e
"${cmd[@]}" | tee "${out}/stdout.txt"
echo "${PIPESTATUS[0]}" >"${out}/exit_code.txt"
set -e
cargo xtask assert-headline "${modality}" "${out}/results.jsonl" ${assert_args[@]+"${assert_args[@]}"} \
  | tee "${out}/assert.txt"
exit "${PIPESTATUS[0]}"
