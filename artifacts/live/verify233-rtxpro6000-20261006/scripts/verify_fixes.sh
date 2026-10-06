#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
# Metrum AI Bench CLI: live verification of the #233 fixes (runs on the GPU host, LLM server up on :8000).
set -uo pipefail
R=/opt/metrum-bench; B=$R/bin; OUT=$R/live-results/verify233; mkdir -p "$OUT"; cd "$R"
EP=http://127.0.0.1:8000/v1/chat/completions; MODEL=Qwen/Qwen3-8B
log() { echo "# $(date -u +%H:%M:%SZ) $*"; }
SUT=$R/live-results/serve-llm/sut.json
jq '.extra |= with_entries(.value |= tostring)' "$SUT" > "$OUT/sut.json"

python3 - "$OUT/telemetry.yaml" <<'PY'
import sys, yaml
a = yaml.safe_load(open("docs/telemetry/examples/all-smi.yaml"))
v = yaml.safe_load(open("docs/telemetry/examples/vllm.yaml"))
names = {s["name"] for s in v["sources"]}
a["sources"] = [s for s in a["sources"] if s["name"] not in names] + v["sources"]
yaml.safe_dump(a, open(sys.argv[1], "w"), sort_keys=False)
PY
source scripts/live/lib/hub_prompts.sh
BENCH_BIN_DIR=$B hub_prompts_extract "$R" "$OUT/prompts.jsonl" "$OUT/prompts-report.json" 512 2>/dev/null
{ echo "binary: $B"; "$B/metrum-ai-bench-cli-llm" --version; } > "$OUT/binary.txt"
NOTHINK='{"chat_template_kwargs":{"enable_thinking":false}}'
# The dispatcher needs glibc 2.39; this host is Ubuntu 22.04 (2.35), so run it in an ubuntu:24.04 container on the host network.
DISPATCH=(docker run --rm --network host -v "$B:/b:ro" ubuntu:24.04 /b/metrum-ai-bench-cli)
"${DISPATCH[@]}" --version > "$OUT/dispatcher-version.txt" 2>&1

log "#230 preflight against Qwen3 with thinking ON (default) and OFF"
"${DISPATCH[@]}" preflight --url "$EP" --api-key dummy --model "$MODEL" > "$OUT/f230-preflight-thinking.txt" 2>&1; echo "exit=$?" >> "$OUT/f230-preflight-thinking.txt"
"${DISPATCH[@]}" preflight --url "$EP" --api-key dummy --model "$MODEL" --extra-body-json "$NOTHINK" > "$OUT/f230-preflight-nothink.txt" 2>&1; echo "exit=$?" >> "$OUT/f230-preflight-nothink.txt"

log "#226 warmup barrier: c=16 with only 4 warmup requests (warmup < concurrency)"
"$B/metrum-ai-bench-cli-llm" --url "$EP" --api-key dummy --model "$MODEL" --mode chat --streaming --scenario verify-226 \
  --prompts "$OUT/prompts.jsonl" --num-requests 256 --warmup-requests 4 --concurrency 16 --max-tokens 64 \
  --extra-body-json "$NOTHINK" --sut "$OUT/sut.json" --require-sut --telemetry "$OUT/telemetry.yaml" \
  --ndjson "$OUT/f226.ndjson" --data-log "$OUT/f226.jsonl" > "$OUT/f226.stdout" 2> "$OUT/f226.stderr"; echo "f226 exit=$?"

log "#227 live: --require-telemetry abort (all-smi killed mid-run), then ONE SIGINT; summary must still be written"
"$B/metrum-ai-bench-cli-llm" --url "$EP" --api-key dummy --model "$MODEL" --mode chat --streaming --scenario verify-227 \
  --prompts "$OUT/prompts.jsonl" --num-requests 4000 --warmup-requests 4 --concurrency 4 --max-tokens 256 \
  --extra-body-json "$NOTHINK" --telemetry "$OUT/telemetry.yaml" --require-telemetry --require-telemetry-failures 3 \
  --ndjson "$OUT/f227.ndjson" --data-log "$OUT/f227.jsonl" > "$OUT/f227.stdout" 2> "$OUT/f227.stderr" &
pid=$!
sleep 8; pkill -f 'all-smi api' ; log "all-smi stopped"
sleep 6; kill -INT $pid; log "sent one SIGINT to $pid"
wait $pid; echo "f227 exit=$?" | tee -a "$OUT/f227.exit"
nohup env LD_LIBRARY_PATH=$HOME/.local/bin $HOME/.local/bin/all-smi api --port 9090 --interval 1 > ~/all-smi.log 2>&1 &
sleep 4; curl -fsS http://127.0.0.1:9090/metrics | grep -c '^all_smi_' > "$OUT/allsmi-restarted.txt"

log "#232 #224 #231 LLM strategic sweep c=1..64 (7 stages) + analyze.py"
"$B/metrum-ai-bench-cli-strategic" --url "$EP" --api-key dummy --model "$MODEL" --kind chat --streaming \
  --prompts "$OUT/prompts.jsonl" --max-tokens 64 --sweep-by concurrency --sweep 1,2,4,8,16,32,64 \
  --requests-per-stage 128 --warmup-requests 16 --extra-body-json "$NOTHINK" \
  --telemetry "$OUT/telemetry.yaml" --ndjson "$OUT/llm-sweep.ndjson" --csv "$OUT/llm-sweep.csv" \
  > "$OUT/llm-sweep.stdout.json" 2> "$OUT/llm-sweep.stderr"; echo "llm sweep exit=$?"
python3 docs/queries/analyze.py "$OUT/llm-sweep.ndjson" "$OUT/llm-sweep.stdout.json" > "$OUT/analyze.txt" 2>&1
python3 docs/queries/analyze.py --json "$OUT/llm-sweep.ndjson" "$OUT/llm-sweep.stdout.json" > "$OUT/analyze.json" 2>/dev/null || true

log "#216 launcher override gating (offline sut subcommand)"
{ HF_HUB_OFFLINE=1 IMAGE=vllm/vllm-openai:v0.31.0 bash scripts/live/serve/llm.sh sut >/dev/null 2>&1; echo "IMAGE override without notes: exit=$? (expect non-zero)";
  HF_HUB_OFFLINE=1 IMAGE=vllm/vllm-openai:v0.31.0 SUT_NOTES_OVERRIDE=x SOURCES_OVERRIDE=y bash scripts/live/serve/llm.sh sut >/dev/null 2>&1; echo "IMAGE override with notes+sources: exit=$? (expect 0)";
  HF_HUB_OFFLINE=1 bash scripts/live/serve/llm.sh sut >/dev/null 2>&1; echo "default: exit=$? (expect 0)"; } > "$OUT/f216.txt" 2>&1
log "done"; ls -la "$OUT"
