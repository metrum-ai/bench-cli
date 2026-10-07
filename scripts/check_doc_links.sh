#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Check that relative Markdown links in the selected documents resolve to
# files or directories that exist in this checkout. Absolute URLs, anchors,
# and mail links are ignored.
#
# Usage:
#   scripts/check_doc_links.sh [--strict] [FILE ...]
#
# Without FILE arguments the default documentation set is checked. Missing
# source files are skipped. With --strict, links to the sibling-commit agent
# documents are failures too; without it they are reported as pending.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

STRICT=0
FILES=()
for arg in "$@"; do
  case "${arg}" in
    --strict) STRICT=1 ;;
    -h | --help)
      sed -n '3,15p' "$0"
      exit 0
      ;;
    *) FILES+=("${arg}") ;;
  esac
done

DEFAULT_FILES=(
  README.md
  AGENTS.md
  llms.txt
  PROPOSALS.md
  CHANGELOG.md
  scripts/live/README.md
  docs/INTAKE.md
  docs/AGENT_RUNBOOK.md
  docs/PITFALLS.md
  docs/PROMPT_LIBRARY.md
  docs/STRATEGIC_BENCHMARKING.md
)
if [[ ${#FILES[@]} -eq 0 ]]; then
  FILES=("${DEFAULT_FILES[@]}")
fi

# Paths authored in sibling commits. They are linked from this commit but may
# not exist yet on an isolated branch. Remove an entry once it is committed on
# the same branch as the link.
PENDING_TARGETS=(
  AGENTS.md
  llms.txt
  docs/INTAKE.md
  docs/AGENT_RUNBOOK.md
  docs/PITFALLS.md
  examples/agent/host_bootstrap.sh
  examples/agent/build_prompt_pool.py
  examples/agent/length_check.py
  examples/agent/telemetry.yaml.example
  examples/agent/intake.example.yaml
)

is_pending() {
  local target="$1"
  local entry
  for entry in "${PENDING_TARGETS[@]}"; do
    [[ "${target}" == "${entry}" ]] && return 0
  done
  return 1
}

status=0
checked=0
pending=0
for file in "${FILES[@]}"; do
  if [[ ! -f "${file}" ]]; then
    echo "check_doc_links: skip missing ${file}"
    continue
  fi
  dir="$(dirname "${file}")"
  while IFS= read -r target; do
    [[ -z "${target}" ]] && continue
    case "${target}" in
      http://* | https://* | mailto:* | \#* | data:*) continue ;;
    esac
    # Strip anchor and query, then URL-decode the common escapes.
    clean="${target%%#*}"
    clean="${clean%%\?*}"
    clean="${clean//%20/ }"
    [[ -z "${clean}" ]] && continue
    if [[ "${clean}" == /* ]]; then
      resolved="${ROOT}${clean}"
    else
      resolved="$(cd "${dir}" && pwd)/${clean}"
    fi
    if [[ -e "${resolved}" ]]; then
      checked=$((checked + 1))
      continue
    fi
    rel="${resolved#"${ROOT}"/}"
    if is_pending "${rel}"; then
      echo "check_doc_links: pending ${file}: ${target} (lands in a sibling commit)"
      pending=$((pending + 1))
      if [[ "${STRICT}" -eq 1 ]]; then
        status=1
      fi
      continue
    fi
    echo "check_doc_links: BROKEN ${file}: ${target} -> ${rel}" >&2
    status=1
  done < <(
    python3 - "${file}" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")
for match in re.finditer(r"\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)", text):
    print(match.group(1))
PY
  )
done

echo "check_doc_links: checked ${checked}, pending ${pending}"
exit "${status}"
