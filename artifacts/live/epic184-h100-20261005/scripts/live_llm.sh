#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
# Metrum AI Bench CLI: epic #184 live validation, LLM lane (runs on the GPU host).
set -euo pipefail
R=/opt/metrum-bench
B=$R/bin
OUT=$R/live-results/epic184-llm
mkdir -p "$OUT"
cd "$R"
URL=http://127.0.0.1:8000/v1/chat/completions
MODEL=Qwen/Qwen3-8B
SUT_SRC=$R/live-results/serve-llm/sut.json

# One telemetry YAML with both sources (all-smi fork + vLLM), built from the shipped examples.
python3 - "$OUT/telemetry.yaml" <<'PY'
import sys, yaml
a = yaml.safe_load(open("docs/telemetry/examples/all-smi.yaml"))
v = yaml.safe_load(open("docs/telemetry/examples/vllm.yaml"))
vnames = {s["name"] for s in v["sources"]}
a["sources"] = [s for s in a["sources"] if s["name"] not in vnames] + v["sources"]
print("telemetry sources:", [s["name"] for s in a["sources"]])
yaml.safe_dump(a, open(sys.argv[1], "w"), sort_keys=False)
PY

# Hub prompts (metrum-ai/prompt-library, config sample, profile chat-short), stamped into the SUT.
source scripts/live/lib/hub_prompts.sh
BENCH_BIN_DIR=$B hub_prompts_extract "$R" "$OUT/prompts.jsonl" "$OUT/prompts-report.json" 512
cp "$SUT_SRC" "$OUT/sut.json"
hub_prompts_stamp_sut "$OUT/sut.json" "$OUT/prompts-report.json"
jq '.extra |= with_entries(.value |= tostring)' "$OUT/sut.json" > "$OUT/sut.tmp" && mv "$OUT/sut.tmp" "$OUT/sut.json"
{ echo "binary: $B/metrum-ai-bench-cli-llm"; "$B/metrum-ai-bench-cli-llm" --version; } > "$OUT/binary.txt"

common=(--url "$URL" --api-key dummy --model "$MODEL" --mode chat --streaming
        --prompts "$OUT/prompts.jsonl" --sut "$OUT/sut.json" --require-sut
        --telemetry "$OUT/telemetry.yaml")

echo "== R1 plain (thinking off), c=16, 512 requests"
"$B/metrum-ai-bench-cli-llm" "${common[@]}" --scenario epic184-plain \
  --num-requests 512 --warmup-requests 32 --concurrency 16 --max-tokens 64 \
  --extra-body-json '{"chat_template_kwargs":{"enable_thinking":false}}' \
  --data-log "$OUT/r1-plain.jsonl" --ndjson "$OUT/r1-plain.ndjson" > "$OUT/r1-plain.stdout" 2> "$OUT/r1-plain.stderr"

echo "== R2 reasoning (thinking on), c=8, 64 requests"
"$B/metrum-ai-bench-cli-llm" "${common[@]}" --scenario epic184-reasoning \
  --num-requests 64 --warmup-requests 8 --concurrency 8 --max-tokens 2048 \
  --data-log "$OUT/r2-reasoning.jsonl" --ndjson "$OUT/r2-reasoning.ndjson" > "$OUT/r2-reasoning.stdout" 2> "$OUT/r2-reasoning.stderr"

echo "== R3 strategic chat sweep, concurrency 1..64 (7 stages)"
"$B/metrum-ai-bench-cli-strategic" --url "$URL" --api-key dummy --model "$MODEL" --kind chat --streaming \
  --prompts "$OUT/prompts.jsonl" --max-tokens 64 --sweep-by concurrency --sweep 1,2,4,8,16,32,64 \
  --requests-per-stage 128 --warmup-requests 16 \
  --extra-body-json '{"chat_template_kwargs":{"enable_thinking":false}}' \
  --telemetry "$OUT/telemetry.yaml" --ndjson "$OUT/r3-sweep.ndjson" \
  > "$OUT/r3-sweep.stdout.json" 2> "$OUT/r3-sweep.stderr"

echo "== R4 analyze.py on the sweep"
python3 docs/queries/analyze.py "$OUT/r3-sweep.ndjson" "$OUT/r3-sweep.stdout.json" > "$OUT/r4-analyze.txt" 2>&1 || echo "analyze.py exit $?"

echo "== R5 dispatcher selftest and preflight"
"$B/metrum-ai-bench-cli" selftest > "$OUT/r5-selftest.txt" 2>&1 || echo "selftest exit $?"
"$B/metrum-ai-bench-cli" preflight --url "$URL" --api-key dummy --model "$MODEL" > "$OUT/r5-preflight.txt" 2>&1 || echo "preflight exit $?"

echo "done: $OUT"
ls -la "$OUT"
