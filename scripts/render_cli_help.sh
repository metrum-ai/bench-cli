#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Regenerate docs/CLI.md from clap --help for shipped public CLI binaries.
# Requires built debug binaries under target/debug/.
#
# Usage:
#   scripts/render_cli_help.sh           # rewrite docs/CLI.md
#   scripts/render_cli_help.sh --check   # diff docs/CLI.md against a fresh
#                                        # render; exit 1 on drift

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${ROOT}/docs/CLI.md"
BIN_DIR="${ROOT}/target/debug"

MODE="write"
case "${1:-}" in
  "") ;;
  --check) MODE="check" ;;
  -h | --help)
    sed -n '3,12p' "$0"
    exit 0
    ;;
  *) echo "error: unknown argument: $1" >&2; exit 2 ;;
esac

die() { echo "error: $*" >&2; exit 1; }

BINS=(
  metrum-ai-bench-cli
  metrum-ai-bench-cli-llm
  metrum-ai-bench-cli-vlm
  metrum-ai-bench-cli-asr
  metrum-ai-bench-cli-imagegen
  metrum-ai-bench-cli-prompts
  metrum-ai-bench-cli-strategic
  metrum-ai-bench-cli-mock-server
)

for bin in "${BINS[@]}"; do
  [[ -x "${BIN_DIR}/${bin}" ]] || die "missing ${BIN_DIR}/${bin}; run: cargo build --bins"
done

render_help() {
  local bin="$1"
  shift
  # Skip ASCII banner / preamble; clap usage starts at "Usage:"
  # Unset API key env vars so clap does not embed live secrets into docs/CLI.md.
  env -u OPENAI_API_KEY -u METRUM_AI_BENCH_API_KEY \
    "${BIN_DIR}/${bin}" "$@" --help 2>/dev/null \
      | sed -n '/^Usage:/,$p' \
      | sed 's/[[:space:]]*$//'
}

render_all() {
  local target="$1"
  {
    echo '<!-- Copyright (c) 2026 Metrum AI, Inc. -->'
    echo '<!-- SPDX-License-Identifier: Apache-2.0 -->'
    echo
    echo '# CLI reference'
    echo
    echo 'Generated from `metrum-ai-bench-cli*` `--help`. Re-run'
    echo '`scripts/render_cli_help.sh` after flag changes. Live `--help` is'
    echo 'authoritative if this file drifts.'
    echo
    for bin in "${BINS[@]}"; do
      echo "## \`${bin}\`"
      echo
      echo '```text'
      render_help "${bin}"
      echo '```'
      echo
      if [[ "${bin}" == "metrum-ai-bench-cli" ]]; then
        echo "### \`${bin} preflight\`"
        echo
        echo '```text'
        render_help "${bin}" preflight
        echo '```'
        echo
      fi
    done
  } >"${target}"
}

if [[ "${MODE}" == "check" ]]; then
  [[ -f "${OUT}" ]] || die "missing ${OUT}; run: scripts/render_cli_help.sh"
  tmp="$(mktemp)"
  trap 'rm -f "${tmp}"' EXIT
  render_all "${tmp}"
  if ! diff -u "${OUT}" "${tmp}"; then
    echo "error: ${OUT} is stale; run: scripts/render_cli_help.sh" >&2
    exit 1
  fi
  echo "render_cli_help: ${OUT} is current"
  exit 0
fi

render_all "${OUT}"
echo "wrote ${OUT}"
