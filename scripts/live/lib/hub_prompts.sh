# shellcheck shell=bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Metrum AI Bench CLI live-smoke helper: extract LLM prompts from a Hugging
# Face prompt dataset with metrum-ai-bench-cli-prompts instead of writing
# handmade JSONL. Source this file; it defines functions only.
#
# Defaults (override with environment variables):
#   PROMPT_DATASET   metrum-ai/prompt-library  (public, Apache-2.0)
#   PROMPT_CONFIG    sample                    (never vendor `full` into git)
#   PROMPT_REVISION  main                      (resolved to a commit SHA in the report)
#   PROMPT_PROFILE   chat-short                (256 / 64 tokens)
#   PROMPT_SEED      7
#   BENCH_BIN_DIR    directory holding the bench binaries (checked first; see
#                    scripts/live/lib/bench_bin.sh for the full order)
# An explicit source wins over the Hub defaults:
#   PROMPT_LOCAL_JSONL=/path/rows.jsonl    -> --local-jsonl
#   PROMPT_LOCAL_PARQUET=/path/shard.parquet -> --local-parquet
# Any schema-compatible metrum-ai Hub set works as PROMPT_DATASET, for example
# one of the manipulation-resistant-prompts-* sets for fixed word counts.

hub_prompts_bin() {
  # Prebuilt binaries only; see scripts/live/lib/bench_bin.sh for the order.
  # shellcheck source=scripts/live/lib/bench_bin.sh
  source "$(dirname "${BASH_SOURCE[0]}")/bench_bin.sh"
  bench_bin_resolve "$1" metrum-ai-bench-cli-prompts
}

# hub_prompts_extract REPO_ROOT OUT_JSONL REPORT_JSON COUNT [extra prompts args...]
# Extra args such as `--isl-target 1024 --osl-target 1024` replace the profile.
hub_prompts_extract() {
  local root="$1" out="$2" report="$3" count="$4"
  shift 4
  local bin
  bin="$(hub_prompts_bin "${root}")" || return 1
  local args=(--quiet --count "${count}" --seed "${PROMPT_SEED:-7}"
              --output "${out}" --report "${report}")
  if [[ -n "${PROMPT_LOCAL_JSONL:-}" ]]; then
    args+=(--local-jsonl "${PROMPT_LOCAL_JSONL}")
  elif [[ -n "${PROMPT_LOCAL_PARQUET:-}" ]]; then
    args+=(--local-parquet "${PROMPT_LOCAL_PARQUET}")
  else
    local rev="${PROMPT_REVISION:-main}"
    args+=(--dataset "${PROMPT_DATASET:-metrum-ai/prompt-library}"
           --config "${PROMPT_CONFIG:-sample}"
           --revision "${rev}")
    # Needed by 1.4.x for floating refs; a documented no-op from 1.5.1 on.
    [[ "${rev}" =~ ^[0-9a-f]{40}$ ]] || args+=(--allow-moving-revision)
  fi
  local has_target=0 a
  for a in "$@"; do
    [[ "${a}" == --isl-target || "${a}" == --osl-target || "${a}" == --profile ]] && has_target=1
  done
  [[ "${has_target}" -eq 1 ]] || args+=(--profile "${PROMPT_PROFILE:-chat-short}")
  mkdir -p "$(dirname "${out}")" "$(dirname "${report}")"
  "${bin}" "${args[@]}" "$@" >&2
}

# hub_prompts_note REPORT_JSON -> one line for SUT notes:
# dataset, resolved revision SHA, config, profile, and selected row count.
hub_prompts_note() {
  jq -r '"prompts: dataset=\(.dataset // "local") revision=\(.revision // "n/a") config=\(.config // "n/a") profile=\(.profile.name // "custom")\(if .profile.version then " v\(.profile.version)" else "" end) rows=\(.selected_count) recommended_max_tokens=\(.recommended_max_tokens)"' "$1"
}

# hub_prompts_stamp_sut SUT_JSON REPORT_JSON: append the note to SUT `notes`
# and record the mix under string-valued extra.prompt_* keys (the SUT schema
# requires extra to map strings to strings).
hub_prompts_stamp_sut() {
  local sut="$1" report="$2" note tmp
  note="$(hub_prompts_note "${report}")"
  tmp="$(mktemp)"
  jq --arg note "${note}" --slurpfile mix "${report}" \
    '($mix[0]) as $m
     | .notes = ((.notes // "") + (if (.notes // "") == "" then "" else "; " end) + $note)
     | .extra = ((.extra // {}) + {
         prompt_dataset: ($m.dataset // "local" | tostring),
         prompt_revision: ($m.revision // "n/a" | tostring),
         prompt_config: ($m.config // "n/a" | tostring),
         prompt_profile: ($m.profile.name // "custom" | tostring),
         prompt_rows: ($m.selected_count | tostring),
         prompt_schedule_sha256: ($m.schedule_sha256 // "n/a" | tostring)})' \
    "${sut}" >"${tmp}" && mv "${tmp}" "${sut}"
}
