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

if [[ "${1:-}" == "--check" ]]; then
  exec cargo test --locked --test data_points -- --exact data_points_doc_is_current
fi
METRUM_BENCH_BLESS_DATA_POINTS=1 \
  cargo test --locked --test data_points -- --exact data_points_doc_is_current
echo "wrote docs/DATA_POINTS.md"
