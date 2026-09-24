#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Remote helper: start a long strategic stage, SIGINT mid-run, assert partial summary.
set -uo pipefail

cd /tmp/metrum-e2e
rm -f run-interrupt.ndjson interrupt.pid stdout-interrupt.json interrupt.stderr

./bin/metrum-ai-bench-cli-strategic \
  --url http://127.0.0.1:8000/v1/chat/completions \
  --api-key dummy \
  --model sut \
  --streaming \
  --prompts /tmp/metrum-e2e/prompts/mix.jsonl \
  --max-tokens 256 \
  --ignore-eos \
  --warmup-requests 0 \
  --requests-per-stage 80 \
  --sweep 1 \
  --sweep-by concurrency \
  --sut /tmp/metrum-e2e/sut.json --require-sut \
  --telemetry /tmp/metrum-e2e/telemetry.yaml \
  --ndjson /tmp/metrum-e2e/run-interrupt.ndjson \
  --html /tmp/metrum-e2e/report-interrupt.html \
  --csv /tmp/metrum-e2e/requests-interrupt.csv \
  > /tmp/metrum-e2e/stdout-interrupt.json 2>/tmp/metrum-e2e/interrupt.stderr &

echo $! > /tmp/metrum-e2e/interrupt.pid
pid="$(cat /tmp/metrum-e2e/interrupt.pid)"
echo "interrupt pid=${pid}"

has_request_rows() {
  [[ -f /tmp/metrum-e2e/run-interrupt.ndjson ]] \
    && grep -qE '"kind"[[:space:]]*:[[:space:]]*"request"' /tmp/metrum-e2e/run-interrupt.ndjson
}

has_summary_row() {
  [[ -f /tmp/metrum-e2e/run-interrupt.ndjson ]] \
    && grep -qE '"kind"[[:space:]]*:[[:space:]]*"summary"' /tmp/metrum-e2e/run-interrupt.ndjson
}

for _ in $(seq 1 180); do
  if has_request_rows; then
    sleep 5
    break
  fi
  if ! kill -0 "${pid}" 2>/dev/null; then
    echo "interrupt target exited before request rows; stderr:" >&2
    tail -n 80 /tmp/metrum-e2e/interrupt.stderr >&2 || true
    break
  fi
  sleep 1
done

if kill -0 "${pid}" 2>/dev/null; then
  echo "sending SIGINT to ${pid}"
  kill -INT "${pid}" || true
  # Allow strategic to flush a partial summary before we give up.
  for _ in $(seq 1 120); do
    if has_summary_row; then
      break
    fi
    if ! kill -0 "${pid}" 2>/dev/null; then
      break
    fi
    sleep 1
  done
  if kill -0 "${pid}" 2>/dev/null; then
    echo "summary not yet written; sending SIGTERM" >&2
    kill -TERM "${pid}" || true
    wait "${pid}" || true
  else
    wait "${pid}" || true
  fi
else
  echo "interrupt target not running at SIGINT time" >&2
fi

python3 - <<'PY'
import json, sys
path = "/tmp/metrum-e2e/run-interrupt.ndjson"
partial = False
kinds = set()
try:
    with open(path, encoding="utf-8") as f:
        for line in f:
            row = json.loads(line)
            kinds.add(row.get("kind"))
            if row.get("kind") == "summary":
                partial = bool(row.get("partial"))
except FileNotFoundError:
    sys.exit("missing interrupt ndjson")
print("interrupt ok partial=%s kinds=%s" % (partial, sorted(k for k in kinds if k)))
if "request" not in kinds and "telemetry" not in kinds:
    sys.exit("interrupt ndjson missing request/telemetry rows")
if not partial:
    sys.exit("expected summary.partial=true after SIGINT")
PY
