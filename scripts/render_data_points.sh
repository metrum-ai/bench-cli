#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Regenerate docs/DATA_POINTS.md (Metrum AI Bench data-point counts) from the
# serde schemas. The generator is tests/data_points.rs; without the bless
# variable the same test fails when the committed file is stale (#202).
#
#   scripts/render_data_points.sh           # rewrite docs/DATA_POINTS.md
#   scripts/render_data_points.sh --check   # fail if it is stale

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

TEST=data_points_doc_is_current
# --exact matching nothing exits 0, so require the test to have run.
run() {
  local out
  out="$("$@" cargo test --locked --test data_points -- --exact "${TEST}" 2>&1)" || {
    echo "${out}" >&2
    return 1
  }
  grep -q "test ${TEST} ... ok" <<<"${out}" || {
    echo "${out}" >&2
    echo "error: ${TEST} did not run" >&2
    return 1
  }
}

if [[ "${1:-}" == "--check" ]]; then
  run env
  echo "docs/DATA_POINTS.md is current"
else
  run env METRUM_BENCH_BLESS_DATA_POINTS=1
  echo "wrote docs/DATA_POINTS.md"
fi
