#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
CERT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=common.sh
source "$CERT_DIR/common.sh"
CERT_CASES=(BC-PERF-002)
rc=0; cert_parse P2 "$@" || rc=$?
[[ $rc == 10 ]] && exit 0
[[ $rc == 0 ]] || exit "$rc"
: "${CERT_HF_HUB_CACHE:?P2 requires a prepopulated offline model hub cache}"
CERT_AUX_OWNER="$(python3 -c 'import secrets; print(secrets.token_hex(16))')"
cert_track_owner auxiliary "$CERT_AUX_OWNER" || exit 1
export PERF_OWNER="$CERT_AUX_OWNER"
cert_run python3 "$CERT_DIR/perf_p2_runner.py" --out "$CERT_OUT" --candidate "$CERT_CANDIDATE" \
  --baseline "$CERT_BASELINE" --manifest "$CERT_MANIFEST" --sha "$CERT_SHA" --model "$CERT_MODEL" \
  --revision "$CERT_REVISION" --image "$CERT_IMAGE" --start-utc "$CERT_START_UTC" \
  --owner "$(cert_manifest owner)" --evidence-link "$(cert_manifest evidence_link)"
