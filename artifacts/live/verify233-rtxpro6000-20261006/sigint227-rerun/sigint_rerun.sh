#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
# Metrum AI Bench CLI #227 rerun: SIGINT DURING the drain after a --require-telemetry abort (client-side path).
set -uo pipefail
B=<HOME>/src/bench-cli/target/rel-user/release
O=<SCRATCH>/sigint227
rm -rf "$O"; mkdir -p "$O/metrics"; cd "$O"
printf '# TYPE demo_gauge gauge\ndemo_gauge 1\n' > metrics/metrics
python3 -m http.server 48190 --bind 127.0.0.1 --directory "$O/metrics" > tele.log 2>&1 & TP=$!
"$B/metrum-ai-bench-cli-mock-server" --listen 127.0.0.1:48180 --latency-ms 6000 > mock.log 2>&1 & MP=$!
sleep 1
cat > telemetry.yaml <<'Y'
default_interval_ms: 500
timeout_ms: 400
sources:
  - name: demo
    url: http://127.0.0.1:48190/metrics
    include: ["^demo_gauge$"]
Y
printf '{"prompt":"hello"}\n' > prompts.jsonl
run() { # name, number of SIGINTs
  "$B/metrum-ai-bench-cli-llm" --url http://127.0.0.1:48180/v1/chat/completions --api-key dummy --model mock --mode chat \
    --scenario sigint-$1 --prompts prompts.jsonl --num-requests 200 --concurrency 4 --max-tokens 16 \
    --telemetry telemetry.yaml --require-telemetry --require-telemetry-failures 3 \
    --ndjson $1.ndjson --data-log $1.jsonl > $1.stdout 2> $1.stderr & P=$!
  sleep 2; kill $TP; echo "$(date -u +%T.%3N) telemetry source stopped" >> $1.timeline
  for i in $(seq 1 100); do grep -q 'consecutive scrapes' $1.stderr && break; sleep 0.1; done
  echo "$(date -u +%T.%3N) abort line seen: $(grep -m1 'consecutive scrapes' $1.stderr | cut -c1-120)" >> $1.timeline
  sleep 0.5
  for n in $(seq 1 $2); do kill -0 $P 2>/dev/null && echo "$(date -u +%T.%3N) process alive, sending SIGINT #$n" >> $1.timeline; kill -INT $P; sleep 0.2; done
  wait $P; rc=$?; echo "$(date -u +%T.%3N) exit=$rc" >> $1.timeline
  python3 -m http.server 48190 --bind 127.0.0.1 --directory "$O/metrics" > tele.log 2>&1 & TP=$!; sleep 1
}
run A 1
run B 2
kill $TP $MP 2>/dev/null
for r in A B; do
  echo "== $r"; cat $r.timeline
  echo "summary.v3 in data log: $(grep -c 'summary.v3' $r.jsonl 2>/dev/null)"
  echo "ndjson last row: $(tail -1 $r.ndjson 2>/dev/null | cut -c1-160)"
done
