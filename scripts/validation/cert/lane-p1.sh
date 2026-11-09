#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
CERT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=common.sh
source "$CERT_DIR/common.sh"
# BC-COMP-001 is produced only by the separate two-model barrier.
CERT_CASES=(BC-PERF-001 BC-TELEM-001)
rc=0; cert_parse P1 "$@" || rc=$?
[[ $rc == 10 ]] && exit 0
[[ $rc == 0 ]] || exit "$rc"
: "${CERT_HF_HUB_CACHE:?P1 requires a prepopulated offline model hub cache}"
cert_run python3 "$CERT_DIR/perf_all_smi.py" --manifest "$CERT_MANIFEST" --candidate "$CERT_CANDIDATE" \
  --baseline "$CERT_BASELINE" --evidence "$CERT_OUT/evidence"
ALL_SMI="$(realpath "$(cert_manifest all_smi_binary_path)")"
TELEMETRY="$(realpath "$(cert_manifest telemetry_config_path)")"
[[ "$ALL_SMI" != *:* && "$ALL_SMI" != *$'\n'* ]] || { echo 'invalid all-smi binary path' >&2; exit 1; }
CERT_AUX_OWNER="$(python3 -c 'import secrets; print(secrets.token_hex(16))')"
cert_track_owner auxiliary "$CERT_AUX_OWNER" || exit 1
export PERF_OWNER="$CERT_AUX_OWNER"
CERT_CONTAINER="cert-all-smi-${CERT_CONTAINER_OWNER}"
CERT_STARTED=1
# The pinned exporter binds all interfaces inside its container. Only loopback
# is published on the host; process labels are explicitly disabled.
docker run -d --pull=never --name "$CERT_CONTAINER" --label "metrum.cert.owner=$CERT_CONTAINER_OWNER" \
  --gpus "device=$CUDA_VISIBLE_DEVICES" -p 127.0.0.1:9090:9090 --env HOME=/tmp \
  -v "$ALL_SMI:/usr/local/bin/all-smi:ro" --entrypoint /usr/local/bin/all-smi "$CERT_IMAGE" \
  api --port 9090 --interval 1 --processes=false > "$CERT_OUT/evidence/all-smi-container.txt"
ready=0
for ((i=0;i<60;i++)); do
  if curl --fail --silent --max-time 5 http://127.0.0.1:9090/metrics > "$CERT_OUT/evidence/all-smi-ready.prom"; then
    ready=1; break
  fi
  docker inspect "$CERT_CONTAINER" --format '{{.State.Running}}' | grep -qx true || break
  sleep 1
done
[[ $ready == 1 ]] || { echo 'all-smi exporter did not become ready' >&2; exit 1; }
cert_run python3 "$CERT_DIR/perf_p1_runner.py" --out "$CERT_OUT" --candidate "$CERT_CANDIDATE" \
  --baseline "$CERT_BASELINE" --manifest "$CERT_MANIFEST" --sha "$CERT_SHA" --model "$CERT_MODEL" \
  --revision "$CERT_REVISION" --image "$CERT_IMAGE" --start-utc "$CERT_START_UTC" --telemetry "$TELEMETRY" \
  --owner "$(cert_manifest owner)" --evidence-link "$(cert_manifest evidence_link)"
