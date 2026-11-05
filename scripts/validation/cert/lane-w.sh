#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
CERT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=common.sh
source "$CERT_DIR/common.sh"
if [[ "${1:-}" == --plan ]]; then
  exec python3 "$CERT_DIR/lane.py" W --plan
fi
if [[ "${1:-}" == --finalize ]]; then
  shift
  exec python3 "$CERT_DIR/w_finalize.py" "$@"
fi
# Successful collection has no case result until the platform team signs off.
CERT_CASES=()
rc=0; cert_parse W "$@" || rc=$?
[[ $rc == 0 ]] || exit "$rc"
: "${CERT_HF_HUB_CACHE:?W requires a prepopulated offline model hub cache}"
CERT_OFFLINE=1
CERT_CONTAINER_OWNER="$(python3 -c 'import secrets; print(secrets.token_hex(16))')"
cert_start_server
cert_server_logs
cert_run python3 "$CERT_DIR/w_runner.py" --out "$CERT_OUT" --candidate "$CERT_CANDIDATE" \
  --baseline "$CERT_BASELINE" --manifest "$CERT_MANIFEST" --start-utc "$CERT_START_UTC"
