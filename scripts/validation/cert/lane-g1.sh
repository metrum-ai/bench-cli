#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
CERT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=common.sh
source "$CERT_DIR/common.sh"
if [[ "${1:-}" == --plan ]]; then
  exec python3 "$CERT_DIR/lane.py" G1 --plan
fi
CERT_CASES=(BC-GPU-001 BC-GPU-002 BC-GPU-003)
cert_parse G1 "$@"
CERT_PROMPT="$(cert_manifest prompt_default)"
CERT_OWNER="$(cert_manifest owner)"
CERT_EVIDENCE_LINK="$(cert_manifest evidence_link)"
CERT_PROXY_PORT="$(cert_manifest proxy_port)"
# Preserve an honest FAIL for the still-unimplemented transform case.
python3 "$CERT_DIR/failure_writer.py" --out "$CERT_OUT" --id BC-GPU-003 --sha "$CERT_SHA" \
  --start "$CERT_START_UTC" --reason 'case not implemented; no test executed'
cert_start_server
cert_server_logs
cert_run python3 "$CERT_DIR/g1_clock_runner.py" --out "$CERT_OUT" --candidate "$CERT_CANDIDATE" \
  --baseline "$CERT_BASELINE" --manifest "$CERT_MANIFEST" --prompt "$CERT_PROMPT" \
  --sha "$CERT_SHA" --model "$CERT_MODEL" --revision "$CERT_REVISION" --image "$CERT_IMAGE" \
  --start-utc "$CERT_START_UTC" --owner "$CERT_OWNER" --evidence-link "$CERT_EVIDENCE_LINK" \
  --port "$CERT_PORT" --proxy-port "$CERT_PROXY_PORT" || true
cert_run python3 "$CERT_DIR/g1_truncation_runner.py" --out "$CERT_OUT" --candidate "$CERT_CANDIDATE" \
  --baseline "$CERT_BASELINE" --manifest "$CERT_MANIFEST" --prompt "$CERT_PROMPT" \
  --sha "$CERT_SHA" --model "$CERT_MODEL" --revision "$CERT_REVISION" --image "$CERT_IMAGE" \
  --start-utc "$CERT_START_UTC" --owner "$CERT_OWNER" --evidence-link "$CERT_EVIDENCE_LINK" \
  --port "$CERT_PORT" --proxy-port "$CERT_PROXY_PORT" || true
# G1 is incomplete until all three cases have real assertions.
exit 1
