#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Fail if authored source files lack a Copyright line and SPDX-License-Identifier.
# Also enforces product-naming rules (see .naming-allow).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ALLOW_BASENAMES=(
  Cargo.lock
  go.sum
  uv.lock
  alice_clean.txt
  LICENSE
  Cargo.toml
  deny.toml
  rust-toolchain.toml
  pyproject.toml
)

is_allowlisted() {
  local path="$1"
  local base
  base="$(basename "$path")"
  for allowed in "${ALLOW_BASENAMES[@]}"; do
    if [[ "$base" == "$allowed" ]]; then
      return 0
    fi
  done
  case "$path" in
    test-data/*|*.png|*.jpg|*.jpeg|*.gif|*.webp|*.wav|*.mp3|*.ogg|*.flac|*.bin|*.docx)
      return 0
      ;;
    # Sample / launch configs are not treated as authored source for this gate.
    *.yaml|*.yml)
      if [[ "$path" == .github/workflows/* ]]; then
        return 1
      fi
      return 0
      ;;
  esac
  return 1
}

check_headers() {
  local missing=0
  while IFS= read -r -d '' file; do
    rel="${file#./}"
    if is_allowlisted "$rel"; then
      continue
    fi
    case "$rel" in
      target/*|.git/*|scripts/live/*|dummy-model-server/*)
        continue
        ;;
    esac

    if ! grep -qE 'Copyright \(c\) 2026 Metrum AI, Inc\.' "$file"; then
      echo "MISSING copyright: $rel" >&2
      missing=1
      continue
    fi
    if ! grep -qE 'SPDX-License-Identifier:\s*Apache-2.0' "$file"; then
      echo "MISSING SPDX: $rel" >&2
      missing=1
    fi
  done < <(find . -type f \( \
    -name '*.rs' -o -name '*.py' -o -name '*.sh' -o -name '*.yml' \
  \) ! -path './target/*' ! -path './.git/*' -print0)

  if [[ "$missing" -ne 0 ]]; then
    echo "scripts/check_headers.sh failed: authored files must include Copyright (c) 2026 and SPDX-License-Identifier: Apache-2.0" >&2
    return 1
  fi

  echo "check_headers: ok"
  return 0
}

# $1 file, $2 line text - true if .naming-allow covers this hit.
allowlisted() {
  local f="$1" txt="$2" glob re
  [ -f .naming-allow ] || return 1
  while IFS=$'\t' read -r glob re || [ -n "${glob:-}" ]; do
    [[ -z "${glob:-}" || "$glob" == \#* ]] && continue
    # shellcheck disable=SC2053
    [[ "$f" == $glob ]] && echo "$txt" | grep -qE "$re" && return 0
  done < .naming-allow
  return 1
}

# Approved name: Metrum AI Bench CLI.
# Binary/crate names remain metrum-ai-bench. Reject unapproved legacy forms.
check_naming() {
  local rc=0
  local -a forbidden=(
    'MetrumBench'                # never; use metrum-ai-bench / Metrum AI Bench CLI
    'Metrum Bench CLI'           # missing "AI"
    '\bmetrumbench\b'            # old crate name; shims allowlisted
    'Insights CLI'
    'Bench by Metrum'
    'Metrum Smart Bench'
  )
  local files
  files=$(git ls-files \
    | grep -vE '^(CHANGELOG\.md|docs/HISTORY_REWRITE\.md|HISTORY_REWRITE\.md|\.naming-allow)$' \
    | grep -vE '^(scripts/check_headers\.sh|scripts/tests/check_naming_test\.sh)$' \
    | grep -vE '^scripts/tests/gitleaks/' \
    | grep -vE '^docs/reviews/QUALITY_ASSESSMENT_(REPORT|PROMPT)\.md$' \
    | grep -vE '\.(png|mp3|lock)$' || true)
  if [[ -z "$files" ]]; then
    echo "check_naming: no files to scan" >&2
    return 0
  fi
  local pat without_transition
  for pat in "${forbidden[@]}"; do
    while IFS=: read -r f ln txt || [ -n "${f:-}" ]; do
      [ -z "${f:-}" ] && continue
      if [[ "$pat" == "Insights CLI" ]]; then
        without_transition="${txt//Metrum AI Bench CLI, formerly Metrum Insights CLI/}"
        if ! echo "$without_transition" | grep -q 'Insights CLI'; then
          continue
        fi
      fi
      if allowlisted "$f" "$txt"; then continue; fi
      echo "naming: $f:$ln: forbidden form matching /$pat/: $txt" >&2
      rc=1
    done < <(echo "$files" | xargs -r grep -nHE "$pat" 2>/dev/null || true)
  done
  # bare "Metrum Insights" allowed only in the approved transition form
  while IFS=: read -r f ln txt || [ -n "${f:-}" ]; do
    [ -z "${f:-}" ] && continue
    without_transition="${txt//Metrum AI Bench CLI, formerly Metrum Insights CLI/}"
    if ! echo "$without_transition" | grep -q 'Metrum Insights'; then
      continue
    fi
    allowlisted "$f" "$txt" && continue
    echo "naming: $f:$ln: 'Metrum Insights' only allowed as 'Metrum AI Bench CLI, formerly Metrum Insights CLI'" >&2
    rc=1
  done < <(echo "$files" | xargs -r grep -nH 'Metrum Insights' 2>/dev/null || true)
  if [[ "$rc" -eq 0 ]]; then
    echo "check_naming: ok"
  fi
  return "$rc"
}

rc=0
check_headers || rc=1
check_naming || rc=1
exit "$rc"
