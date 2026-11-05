#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
CERT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=common.sh
source "$CERT_DIR/common.sh"
if [[ "${1:-}" == --plan ]]; then
  exec python3 "$CERT_DIR/lane.py" R --plan
fi
CERT_CASES=(BC-REASON-001 BC-REASON-002 BC-REASON-003 BC-REASON-004 BC-HW-001)
cert_parse R "$@"
CERT_PROMPT="$(cert_manifest prompt_default)"
CERT_OWNER="$(cert_manifest owner)"
CERT_EVIDENCE_LINK="$(cert_manifest evidence_link)"
CERT_PROXY_PORT="$(cert_manifest proxy_port)"
cert_start_server
cert_server_logs
cert_run python3 "$CERT_DIR/r_runner.py" --out "$CERT_OUT" --candidate "$CERT_CANDIDATE" \
  --baseline "$CERT_BASELINE" --manifest "$CERT_MANIFEST" --prompt "$CERT_PROMPT" \
  --sha "$CERT_SHA" --model "$CERT_MODEL" --revision "$CERT_REVISION" --image "$CERT_IMAGE" \
  --start-utc "$CERT_START_UTC" --owner "$CERT_OWNER" --evidence-link "$CERT_EVIDENCE_LINK" \
  --port "$CERT_PORT" --proxy-port "$CERT_PROXY_PORT"
