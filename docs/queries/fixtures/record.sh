#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Re-records the analyze.py NDJSON fixtures against the mock server.
# Usage: BIN_DIR=/path/to/target/release docs/queries/fixtures/record.sh
# Writes next to this script:
#   sweep5.{ndjson,stdout.json}: 5 stages through capacity_proxy.py (at most
#     $CAPACITY requests reach the mock at once), so p95 bends past
#     c=$CAPACITY and knee_detection reports a real knee (#232, #240).
#   sweep5_no_bend.{ndjson,stdout.json}: the same sweep straight to the
#     fixed-latency mock; p95 rises less than 20%, so the reason is no_bend.
#   sweep3.{ndjson,stdout.json}: 3 stages, insufficient_points.
# Every stage runs $RPS requests so it spans several telemetry scrapes
# (250 ms interval) and each stage has power and preemption samples.
# The generated .ndjson and .stdout.json fixtures are Copyright (c) 2026
# Metrum AI, Inc., SPDX-License-Identifier: Apache-2.0. They carry no comment
# header because JSON and NDJSON have no comment syntax.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
bin="${BIN_DIR:?set BIN_DIR to a directory with the release binaries}"
work="$(mktemp -d)"
trap 'kill "${mock_pid:-0}" "${proxy_pid:-0}" 2>/dev/null || true; rm -rf "$work"' EXIT
CAPACITY=4
RPS=64

# Older mock builds print the --listen flag, not the bound port, so pick one here.
port="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')"
url="http://127.0.0.1:$port"
"$bin/metrum-ai-bench-cli-mock-server" --listen "127.0.0.1:$port" --telemetry-fixture \
  --latency-ms 200 --fail-every 7 >"$work/mock.log" 2>&1 &
mock_pid=$!
for _ in $(seq 50); do
  curl -fsS "$url/health" >/dev/null 2>&1 && break
  sleep 0.1
done
free_port() {
  python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])'
}
gate_port="$(free_port)"
gate_url="http://127.0.0.1:$gate_port"
python3 "$here/capacity_proxy.py" "$gate_port" "$url" "$CAPACITY" >"$work/proxy.log" 2>&1 &
proxy_pid=$!
for _ in $(seq 50); do
  curl -fsS "$gate_url/health" >/dev/null 2>&1 && break
  sleep 0.1
done
sed "s#MOCK_URL#$url#" "$here/mock-telemetry.yaml" >"$work/telemetry.yaml"

record() {
  local name="$1" sweep="$2" target="$3"
  # Telemetry and --metrics-url always read the mock directly; only the
  # inference requests go to $target. Relative paths: points[].config
  # echoes --ndjson and --telemetry verbatim.
  (cd "$work" && "$bin/metrum-ai-bench-cli-strategic" \
    --url "$target/v1/chat/completions" --model mock --api-key dummy \
    --sweep "$sweep" --sweep-by concurrency --requests-per-stage "$RPS" \
    --max-tokens 16 --warmup-requests 0 \
    --ndjson "$name.ndjson" --telemetry telemetry.yaml \
    --metrics-url "$url/metrics" >"$name.stdout.json")
  # The ports vary per run; pin them so re-records diff cleanly. The
  # trailing slash keeps one port from matching a prefix of the other.
  sed -e "s#$url/#http://127.0.0.1:MOCK_PORT/#g" -e "s#$gate_url/#http://127.0.0.1:GATE_PORT/#g" \
    "$work/$name.ndjson" \
    >"$here/$name.ndjson"
  # Keep only the stdout keys analyze.py reads. environment carries the
  # hostname and strategic has no working --redact-hostname, so it is omitted.
  python3 - "$work/$name.stdout.json" "$here/$name.stdout.json" "$url" "$gate_url" <<'PY'
import json, sys
keep = ("schema_version", "tool_version", "partial", "points", "knee", "knee_detection")
with open(sys.argv[1], encoding="utf-8") as f:
    text = f.read().replace(sys.argv[3] + "/", "http://127.0.0.1:MOCK_PORT/")
    text = text.replace(sys.argv[4] + "/", "http://127.0.0.1:GATE_PORT/")
src = json.loads(text)
out = {k: src[k] for k in keep if k in src}
with open(sys.argv[2], "w", encoding="utf-8") as f:
    json.dump(out, f, indent=2, sort_keys=True)
    f.write("\n")
PY
}

record sweep5 1,2,4,8,16 "$gate_url"
record sweep5_no_bend 1,2,4,8,16 "$url"
record sweep3 1,4,16 "$url"
