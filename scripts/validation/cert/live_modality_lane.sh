#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
CERT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=common.sh
source "$CERT_DIR/common.sh"
LANE=$1 MODALITY=$2 CASE_ID=$3
shift 3
if [[ "${1:-}" == --plan ]]; then
  exec python3 "$CERT_DIR/lane.py" "$LANE" --plan
fi
CERT_CASES=("$CASE_ID")
cert_parse "$LANE" "$@"
CERT_PROMPT="$(cert_manifest prompt_default)"
CERT_OWNER="$(cert_manifest owner)"
CERT_EVIDENCE_LINK="$(cert_manifest evidence_link)"
CERT_SUT="$(cert_manifest sut_path)"
ROOT="$(cd "$CERT_DIR/../../.." && pwd)"
[[ "$(git -C "$ROOT" rev-parse HEAD)" == "$CERT_SHA" ]] || { echo 'xtask checkout does not match candidate commit' >&2; exit 1; }
if ! git -C "$ROOT" diff --quiet || ! git -C "$ROOT" diff --cached --quiet; then
  echo 'xtask checkout has tracked changes' >&2
  exit 1
fi
[[ -n "${CERT_HF_HUB_CACHE:-}" && -d "$CERT_HF_HUB_CACHE" ]] || { echo 'pinned hub cache required for offline serving' >&2; exit 1; }
cert_manifest server_flags > "$CERT_OUT/evidence/server-flags.nul"
mapfile -d '' -t flags < "$CERT_OUT/evidence/server-flags.nul"
server_args=''
for flag in "${flags[@]}"; do
  [[ "$flag" != *[[:space:]]* ]] || { echo 'xtask server flags cannot contain whitespace' >&2; exit 1; }
  server_args+="${flag} "
done
CERT_CONTAINER_OWNER="$(python3 -c 'import secrets; print(secrets.token_hex(16))')"
CACHE="$(realpath "$CERT_HF_HUB_CACHE")"
[[ "$CACHE" != *:* && "$CACHE" != *$'\n'* ]] || { echo 'invalid hub cache path' >&2; exit 1; }
export CARGO_NET_OFFLINE=true GPU_DEVICES="device=${CUDA_VISIBLE_DEVICES}" HF_HOME="$CACHE"
export SERVE_OUT="$CERT_OUT/evidence/serve"
[[ "$(sha256sum "$CERT_MANIFEST" | cut -d' ' -f1)" == "$CERT_MANIFEST_SHA" ]] || { echo 'manifest changed during preflight' >&2; exit 1; }
CERT_STARTED=1
cert_run cargo --locked xtask serve "$MODALITY" start --root "$ROOT" --model "$CERT_MODEL" \
  --image "$CERT_IMAGE" --model-revision "$CERT_REVISION" "--args=$server_args" \
  --notes 'Pinned release certification serving configuration' --sources "https://huggingface.co/$CERT_MODEL" \
  --port "$CERT_PORT" --container-name "$CERT_CONTAINER" --cert-owner "$CERT_CONTAINER_OWNER" \
  --no-replace --loopback-only --hub-cache-only --offline \
  > "$CERT_OUT/evidence/serve-stdout.txt" 2> "$CERT_OUT/evidence/serve-stderr.txt"
docker inspect --format '{{.Id}}' "$CERT_CONTAINER" > "$CERT_OUT/evidence/container-id.txt"
curl --fail --silent --max-time 5 "http://localhost:${CERT_PORT}/v1/models" > "$CERT_OUT/evidence/models.json"
cert_server_logs
cert_run python3 "$CERT_DIR/live_modality_runner.py" --modality "$MODALITY" --out "$CERT_OUT" \
  --candidate "$CERT_CANDIDATE" --baseline "$CERT_BASELINE" --manifest "$CERT_MANIFEST" \
  --prompt "$CERT_PROMPT" --sut "$CERT_SUT" --sha "$CERT_SHA" --model "$CERT_MODEL" \
  --revision "$CERT_REVISION" --image "$CERT_IMAGE" --start-utc "$CERT_START_UTC" \
  --owner "$CERT_OWNER" --evidence-link "$CERT_EVIDENCE_LINK" --port "$CERT_PORT"
[[ "$(sha256sum "$CERT_MANIFEST" | cut -d' ' -f1)" == "$CERT_MANIFEST_SHA" ]] || { echo 'manifest changed during run' >&2; exit 1; }
cert_manifest owner > /dev/null
if ! git -C "$ROOT" diff --quiet || ! git -C "$ROOT" diff --cached --quiet; then
  echo 'xtask checkout changed during run' >&2
  exit 1
fi
