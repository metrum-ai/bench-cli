#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Regenerate the public smoke matrix on a live GPU host and publish the campaign
# manifest under docs/smoke/<campaign-id>/manifest.json.
#
# Requires Shadeform credentials and GPUs. matrix_smoke.sh already passes
# --sut / --require-sut on every cell. Do not run this in CI.
#
# Usage:
#   scripts/live/regen_smoke.sh              # dry-run plan
#   scripts/live/regen_smoke.sh --execute    # full launch + sweep + report + copy

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
MATRIX="${SCRIPT_DIR}/matrix_smoke.sh"
RESULTS_DIR="${RESULTS_DIR:-${REPO_ROOT}/live-results}"

execute=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --execute) execute=1; shift ;;
    -h|--help)
      sed -n '2,14p' "$0"
      exit 0
      ;;
    *)
      echo "usage: $0 [--execute]" >&2
      exit 2
      ;;
  esac
done

campaign_id="${CAMPAIGN_ID:-matrix-$(date -u +%Y%m%d-%H%M%S)}"
export CAMPAIGN_ID="${campaign_id}"
export RESULTS_DIR

echo "campaign_id=${campaign_id}"
echo "results=${RESULTS_DIR}/campaign-${campaign_id}"
echo "manifest_out=${REPO_ROOT}/docs/smoke/${campaign_id}/manifest.json"

if [[ "${execute}" -ne 1 ]]; then
  echo "dry-run: pass --execute to launch, sweep, report, and copy the manifest" >&2
  bash "${MATRIX}" plan
  exit 0
fi

bash "${MATRIX}" --execute launch
bash "${MATRIX}" --execute sweep
bash "${MATRIX}" --execute validate
bash "${MATRIX}" --execute report

src_manifest="${RESULTS_DIR}/campaign-${campaign_id}/manifest.json"
if [[ ! -f "${src_manifest}" ]]; then
  echo "error: expected manifest at ${src_manifest}" >&2
  exit 1
fi

dest_dir="${REPO_ROOT}/docs/smoke/${campaign_id}"
mkdir -p "${dest_dir}"
cp -f "${src_manifest}" "${dest_dir}/manifest.json"
echo "wrote ${dest_dir}/manifest.json"
echo "docs/SMOKE_RESULTS.md updated by matrix_smoke report; review before commit"
