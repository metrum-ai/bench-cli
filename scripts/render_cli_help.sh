#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Regenerate docs/CLI.md from clap --help for the four modality binaries.
# Requires built debug binaries under target/debug/.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${ROOT}/docs/CLI.md"
BIN_DIR="${ROOT}/target/debug"

die() { echo "error: $*" >&2; exit 1; }

for bin in metrum-ai-bench-llm metrum-ai-bench-vlm metrum-ai-bench-asr metrum-ai-bench-imagegen; do
  [[ -x "${BIN_DIR}/${bin}" ]] || die "missing ${BIN_DIR}/${bin}; run: cargo build --bins"
done

{
  echo '<!-- Copyright (c) 2026 Metrum AI, Inc. -->'
  echo '<!-- SPDX-License-Identifier: Apache-2.0 -->'
  echo
  echo '# CLI reference'
  echo
  echo 'Generated from `metrum-ai-bench-* --help`. Re-run'
  echo '`scripts/render_cli_help.sh` after flag changes. Live `--help` is'
  echo 'authoritative if this file drifts.'
  echo
  for name in llm vlm asr imagegen; do
    echo "## \`metrum-ai-bench-${name}\`"
    echo
    echo '```text'
    # Skip the ASCII banner; clap usage starts at "Usage:"
    "${BIN_DIR}/metrum-ai-bench-${name}" --help 2>/dev/null \
      | sed -n '/^Usage:/,$p'
    echo '```'
    echo
  done
} >"${OUT}"

echo "wrote ${OUT}"
