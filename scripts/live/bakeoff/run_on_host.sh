#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# On-host LLM bake-off: metrum-ai-bench-cli vs ai-dynamo/aiperf.
# Run THIS SCRIPT on the Shadeform GPU host (loopback to the server).
# Do not drive load from a laptop.
#
# Codex UX capture (optional): start with
#   asciinema rec -c 'codex' codex-metrum.cast
# after configuring Kimi platform (see scripts/live/bakeoff/CODEX_KIMI.md).
# Pass MOONSHOT_API_KEY via env (never commit).

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
OUT="${BAKEOFF_OUT:-$ROOT/docs/reviews/bakeoff/$(date -u +%Y%m%d)-$(hostname -s)}"
TIMINGS="$OUT/timings.jsonl"
TIMER="$ROOT/scripts/live/bakeoff/phase_timer.sh"
URL="${BENCH_URL:-http://127.0.0.1/v1/chat/completions}"
MODEL="${BENCH_MODEL:-Qwen/Qwen2.5-7B-Instruct}"
API_KEY="${BENCH_API_KEY:-dummy}"
HF_REV="${HF_REV:-0666f62e581b482838ae2e17b333ee36ff3d01b0}"
PROFILE="${PROFILE:-chat-short}"
SWEEP="${SWEEP:-1,2,4,8,16}"
RPS="${REQUESTS_PER_STAGE:-32}"
WARMUP="${WARMUP_REQUESTS:-4}"

mkdir -p "$OUT"
chmod +x "$TIMER" || true
: >"$TIMINGS"

echo "bakeoff out=$OUT url=$URL model=$MODEL" | tee "$OUT/run.log"

phase() {
  local name="$1" tool="$2"
  shift 2
  "$TIMER" "$TIMINGS" "$name" "$tool" -- "$@"
}

phase host_prep shared bash -lc 'nvidia-smi -L; curl -fsS '"$URL"'/../models >/dev/null || curl -fsS http://127.0.0.1/v1/models >/dev/null'

# Install metrum binary (prefer local cargo build artifacts if present)
phase install_metrum metrum bash -lc '
  set -euo pipefail
  if command -v metrum-ai-bench-cli-strategic >/dev/null 2>&1; then
    metrum-ai-bench-cli-strategic --version-only || true
    exit 0
  fi
  if [[ -x "'"$ROOT"'/target/release/metrum-ai-bench-cli-strategic" ]]; then
    export PATH="'"$ROOT"'/target/release:$PATH"
    metrum-ai-bench-cli-strategic --version-only
    exit 0
  fi
  echo "metrum binary not on PATH; build on host or scp release tarball first" >&2
  exit 1
'

phase install_aiperf aiperf bash -lc '
  set -euo pipefail
  python3 -m venv "'"$OUT"'/aiperf-venv"
  # shellcheck disable=SC1091
  source "'"$OUT"'/aiperf-venv/bin/activate"
  pip install -U pip wheel
  pip install "aiperf" || pip install "git+https://github.com/ai-dynamo/aiperf.git"
  aiperf --help >/dev/null || true
  which aiperf
'

phase prompt_extract metrum bash -lc '
  set -euo pipefail
  export PATH="'"$ROOT"'/target/release:$PATH"
  if command -v metrum-ai-bench-cli-prompts >/dev/null 2>&1; then
    PROMPTS_BIN=metrum-ai-bench-cli-prompts
  else
    PROMPTS_BIN="metrum-ai-bench-cli prompts --"
  fi
  # shellcheck disable=SC2086
  $PROMPTS_BIN \
    --dataset metrum-ai/prompt-library \
    --revision "'"$HF_REV"'" \
    --config full \
    --profile "'"$PROFILE"'" \
    --count 32 --seed 42 \
    --output "'"$OUT"'/mix.jsonl" \
    --report "'"$OUT"'/mix-report.json"
'

phase sut_write shared bash -lc '
  cat > "'"$OUT"'/sut.json" <<JSON
{
  "name": "shadeform-bakeoff-$(hostname -s)",
  "provenance": "declared",
  "gpu": {"model": "$(nvidia-smi --query-gpu=name --format=csv,noheader | head -1)", "count": 1},
  "runtime": {"name": "vllm", "config": "vendor docker on-host loopback"},
  "model": {"id": "'"$MODEL"'"},
  "notes": "Bake-off vs ai-dynamo/aiperf; loadgen on host; prompt-library '"$PROFILE"' rev '"$HF_REV"'"
}
JSON
'

MAX_TOKENS="$(jq -r .recommended_max_tokens "$OUT/mix-report.json" 2>/dev/null || echo 128)"

phase metrum_strategic_sweep metrum bash -lc '
  set -euo pipefail
  export PATH="'"$ROOT"'/target/release:$PATH"
  metrum-ai-bench-cli-strategic \
    --url "'"$URL"'" --api-key "'"$API_KEY"'" --model "'"$MODEL"'" \
    --streaming --prompts "'"$OUT"'/mix.jsonl" --max-tokens "'"$MAX_TOKENS"'" \
    --warmup-requests "'"$WARMUP"'" \
    --sweep "'"$SWEEP"'" --requests-per-stage "'"$RPS"'" \
    --sut "'"$OUT"'/sut.json" --require-sut \
    --html "'"$OUT"'/metrum-report.html" \
    --csv "'"$OUT"'/metrum-requests.csv" \
    > "'"$OUT"'/metrum-summary.json"
'

phase aiperf_matched_sweep aiperf bash -lc '
  set -euo pipefail
  # shellcheck disable=SC1091
  source "'"$OUT"'/aiperf-venv/bin/activate"
  aiperf --help > "'"$OUT"'/aiperf-help.txt" 2>&1 || true
  python3 - <<PY
import json, pathlib
src = pathlib.Path("'"$OUT"'/mix.jsonl")
dst = pathlib.Path("'"$OUT"'/aiperf-prompts.jsonl")
lines = []
for line in src.read_text().splitlines():
    if not line.strip():
        continue
    o = json.loads(line)
    lines.append(json.dumps({"text": o["prompt"]}, ensure_ascii=False))
dst.write_text("\n".join(lines) + "\n")
print("wrote", len(lines), "aiperf single_turn prompts")
PY
  BASE_URL="${URL%/v1/chat/completions}"
  BASE_URL="${BASE_URL%/v1}"
  aiperf profile \
    --model "'"$MODEL"'" \
    --url "'"$BASE_URL"'" \
    --endpoint-type chat \
    --streaming \
    --concurrency "'"$SWEEP"'" \
    --request-count "'"$RPS"'" \
    --warmup-request-count "'"$WARMUP"'" \
    --output-tokens-mean "'"$MAX_TOKENS"'" \
    --input-file "'"$OUT"'/aiperf-prompts.jsonl" \
    --custom-dataset-type single_turn \
    --artifact-dir "'"$OUT"'/aiperf-artifacts" \
    > "'"$OUT"'/aiperf-summary.txt" 2>&1
'

phase client_ceiling_dummy metrum bash -lc '
  set -euo pipefail
  export PATH="'"$ROOT"'/target/release:$PATH"
  if command -v metrum-ai-bench-cli-mock-server >/dev/null 2>&1; then
    metrum-ai-bench-cli-mock-server --listen 127.0.0.1:18080 >/tmp/mock-bakeoff.log 2>&1 &
    echo $! > /tmp/mock-bakeoff.pid
    sleep 1
    metrum-ai-bench-cli-strategic \
      --url http://127.0.0.1:18080/v1/chat/completions --api-key dummy --model mock \
      --max-tokens 16 --sweep 32,64,128 --requests-per-stage 200 \
      --csv "'"$OUT"'/ceiling.csv" --html "'"$OUT"'/ceiling.html" \
      > "'"$OUT"'/ceiling-summary.json" || true
    kill "$(cat /tmp/mock-bakeoff.pid)" 2>/dev/null || true
  else
    echo "mock server not available" > "'"$OUT"'/ceiling-summary.json"
  fi
'

echo "bakeoff complete: $OUT" | tee -a "$OUT/run.log"
jq -s '.' "$TIMINGS" >"$OUT/timings.json" || true
