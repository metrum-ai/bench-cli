#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
CERT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$CERT_DIR/common.sh"
[[ "${CERT_CASE_ID:-}" == BC-KV-003 || "${CERT_CASE_ID:-}" == BC-KV-004 ]] || { echo 'select one SGLang KV case' >&2; exit 2; }
CERT_CASES=("$CERT_CASE_ID")
cert_parse KVS "$@"
# Reuse provisional result publication after the common owned-server cleanup.
CERT_PERF_FINALIZE=1
CERT_KV_ARGS=(--out "$CERT_OUT" --candidate "$CERT_CANDIDATE" --baseline "$CERT_BASELINE" --manifest "$CERT_MANIFEST"
              --container "$CERT_CONTAINER" --container-owner "$CERT_CONTAINER_OWNER")
CERT_STARTED=1
python3 "$CERT_DIR/kv_sglang_runner.py" "${CERT_KV_ARGS[@]}" --start-server
for ((attempt=0; attempt<900; attempt++)); do
  if curl --fail --silent --max-time 5 "http://localhost:${CERT_PORT}/v1/models" > "$CERT_OUT/evidence/models.json"; then
    cert_phase server-ready
    cert_server_logs
    cert_run python3 "$CERT_DIR/kv_sglang_runner.py" "${CERT_KV_ARGS[@]}"
    exit 0
  fi
  docker inspect "$CERT_CONTAINER" --format '{{.State.Running}}' | grep -qx true || exit 1
  sleep 2
done
echo 'server readiness timed out' >&2
exit 1
