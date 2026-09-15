#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Fail if authored source files lack a Copyright line and SPDX-License-Identifier.
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

missing=0
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
  exit 1
fi

echo "check_headers: ok"
