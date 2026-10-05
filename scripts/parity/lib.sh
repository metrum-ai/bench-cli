# shellcheck shell=bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Metrum AI Bench CLI parity harness: shared helpers for run_pair.sh and
# run_tele.sh. Source this file; it defines variables and functions only.
# It never compiles and never installs anything unless PARITY_INSTALL_AIPERF=1.

PARITY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PARITY_ROOT="$(cd "${PARITY_DIR}/../.." && pwd)"
# shellcheck source=../live/lib/bench_bin.sh
source "${PARITY_ROOT}/scripts/live/lib/bench_bin.sh"

# Knobs (env). Defaults are the epic #184 setup: c=4, 64 measured + 4 warmup.
CONCURRENCY="${CONCURRENCY:-4}"
REQUESTS="${REQUESTS:-64}"
WARMUP="${WARMUP:-4}"
MAX_TOKENS="${MAX_TOKENS:-64}"
MODEL="${MODEL:-parity-mock}"
MOCK_PORT="${MOCK_PORT:-18080}"
FORK_PORT="${FORK_PORT:-19090}"
MOCK_SEED="${MOCK_SEED:-0}"
# Pacing (ms). See README "Pacing rule"; ITL must stay > 0.
PREFILL_MS="${PREFILL_MS:-40}"
PER_PROMPT_TOKEN_MS="${PER_PROMPT_TOKEN_MS:-0.05}"
ITL_MS="${ITL_MS:-8}"
# AIPerf tokenizes client-side by default (as in the epic run); gpt2 is small
# and public. Any HF id or local tokenizer path works.
AIPERF_TOKENIZER="${AIPERF_TOKENIZER:-gpt2}"
AIPERF_VERSION_PIN="${AIPERF_VERSION_PIN:-0.13.0}"
PYTHON="${PYTHON:-python3}"

PARITY_PIDS=()

parity_log() { echo "[parity $(date -u +%H:%M:%SZ)] $*" >&2; }

parity_cleanup() {
  local pid
  for pid in "${PARITY_PIDS[@]:-}"; do
    [[ -n "${pid}" ]] && kill "${pid}" 2>/dev/null || true
  done
  for pid in "${PARITY_PIDS[@]:-}"; do
    [[ -n "${pid}" ]] && wait "${pid}" 2>/dev/null || true
  done
  PARITY_PIDS=()
}

# parity_stop_last -> stop the most recently started helper (the mock).
parity_stop_last() {
  local n=${#PARITY_PIDS[@]}
  ((n > 0)) || return 0
  kill "${PARITY_PIDS[n - 1]}" 2>/dev/null || true
  wait "${PARITY_PIDS[n - 1]}" 2>/dev/null || true
  unset "PARITY_PIDS[n - 1]"
  PARITY_PIDS=("${PARITY_PIDS[@]}")
}

# parity_want TOOL -> rc 0 when TOOL is in TOOLS (default: bench aiperf)
parity_want() { [[ " ${TOOLS:-bench aiperf} " == *" $1 "* ]]; }

# parity_port_free PORT -> rc 0 when nothing listens on 127.0.0.1:PORT.
# A stale mock left on the port would otherwise be measured instead of ours.
parity_port_free() {
  if "${PYTHON}" -c 'import socket, sys
s = socket.socket()
s.settimeout(0.5)
sys.exit(0 if s.connect_ex(("127.0.0.1", int(sys.argv[1]))) == 0 else 1)' "$1"; then
    echo "error: port $1 already has a listener (stale mock?). Stop it or set MOCK_PORT/FORK_PORT." >&2
    return 1
  fi
}

# parity_wait_http URL PID [MATCH] -> rc 0 once URL answers 200 (and its body
# contains MATCH, when given) while PID, the process we started, is alive.
parity_wait_http() {
  local url="$1" pid="$2" match="${3:-}" i body
  for ((i = 0; i < 50; i++)); do
    if ! kill -0 "${pid}" 2>/dev/null; then
      echo "error: helper for ${url} exited during startup (see its log)" >&2
      return 1
    fi
    if body="$(curl -fsS --max-time 1 "${url}" 2>/dev/null)"; then
      if [[ -z "${match}" || "${body}" == *"${match}"* ]]; then
        kill -0 "${pid}" 2>/dev/null && return 0
      fi
    fi
    sleep 0.1
  done
  echo "error: ${url} did not come up from pid ${pid}" >&2
  return 1
}

# parity_start_mock LOG [mock args...] -> starts the paced mock on MOCK_PORT.
# The mock echoes a per-start nonce on /health, so the wait only succeeds
# against the process started here.
parity_start_mock() {
  local log="$1" nonce
  shift
  parity_port_free "${MOCK_PORT}"
  nonce="parity-$$-${RANDOM}${RANDOM}"
  "${PYTHON}" "${PARITY_DIR}/mock_server.py" --port "${MOCK_PORT}" --model "${MODEL}" \
    --prefill-ms "${PREFILL_MS}" --per-prompt-token-ms "${PER_PROMPT_TOKEN_MS}" \
    --itl-ms "${ITL_MS}" --seed "${MOCK_SEED}" --nonce "${nonce}" "$@" >"${log}" 2>&1 &
  PARITY_PIDS+=("$!")
  parity_wait_http "http://127.0.0.1:${MOCK_PORT}/health" "$!" "${nonce}"
}

# parity_start_fork LOG -> replays the all-smi fork page on FORK_PORT
parity_start_fork() {
  local log="$1"
  parity_port_free "${FORK_PORT}"
  "${PYTHON}" "${PARITY_DIR}/fork_page.py" serve --port "${FORK_PORT}" \
    ${FORK_PAGE:+--page "${FORK_PAGE}"} >"${log}" 2>&1 &
  PARITY_PIDS+=("$!")
  parity_wait_http "http://127.0.0.1:${FORK_PORT}/metrics" "$!"
}

# parity_bench_bin NAME -> path. Default dir is the shared Rust lane build;
# BENCH_BIN_DIR overrides. Never builds.
parity_bench_bin() {
  local default_dir="${PARITY_ROOT}/../bench-cli-rust/target/release"
  if [[ -z "${BENCH_BIN_DIR:-}" && -x "${default_dir}/$1" ]]; then
    BENCH_BIN_DIR="$(cd "${default_dir}" && pwd)"
  fi
  bench_bin_resolve "${PARITY_ROOT}" "$1"
}

# parity_bench_identity BIN -> bench_bin_identity against the checkout that
# holds BIN (for example ../bench-cli-rust), not this worktree.
parity_bench_identity() {
  bench_bin_identity "$1" "$(dirname "$1")"
}

# parity_aiperf -> path to an aiperf CLI. AIPERF wins, then PATH, then
# OUT/.aiperf-venv (created only when PARITY_INSTALL_AIPERF=1).
parity_aiperf() {
  local out="$1" venv
  if [[ -n "${AIPERF:-}" ]]; then echo "${AIPERF}"; return 0; fi
  if command -v aiperf >/dev/null 2>&1; then command -v aiperf; return 0; fi
  venv="${out}/.aiperf-venv"
  if [[ -x "${venv}/bin/aiperf" ]]; then echo "${venv}/bin/aiperf"; return 0; fi
  if [[ "${PARITY_INSTALL_AIPERF:-0}" == 1 ]]; then
    parity_log "installing aiperf==${AIPERF_VERSION_PIN} into ${venv}"
    "${PYTHON}" -m venv "${venv}" >&2
    "${venv}/bin/pip" install -q "aiperf==${AIPERF_VERSION_PIN}" >&2
    echo "${venv}/bin/aiperf"
    return 0
  fi
  echo "error: aiperf not found. Set AIPERF=/path/to/aiperf, put it on PATH, or rerun with" >&2
  echo "       PARITY_INSTALL_AIPERF=1 to pip install aiperf==${AIPERF_VERSION_PIN} into ${venv}." >&2
  return 1
}

# parity_prompts OUT -> writes OUT/prompts.jsonl (bench) and OUT/aiperf-input.jsonl.
# PROMPTS=<jsonl with "prompt"> uses your file (for example a
# metrum-ai-bench-cli-prompts mix); otherwise 16 deterministic prompts of
# 20 to 125 words are generated. Counts do not depend on prompt content.
parity_prompts() {
  local out="$1"
  "${PYTHON}" - "${out}" "${PROMPTS:-}" "${MAX_TOKENS}" <<'PY'
import json, sys
out, src, max_tokens = sys.argv[1], sys.argv[2], int(sys.argv[3])
rows = []
if src:
    with open(src, encoding="utf-8") as fh:
        rows = [json.loads(l)["prompt"] for l in fh if l.strip()]
else:
    # Single-token words (GPT-2), so prompt usage equals client-side ISL.
    words = ("the quick brown fox jumps over a lazy dog while we count "
             "every data point").split()
    rows = [" ".join((words * 20)[: 20 + i * 7]) for i in range(16)]
with open(f"{out}/prompts.jsonl", "w", encoding="utf-8") as b, \
     open(f"{out}/aiperf-input.jsonl", "w", encoding="utf-8") as a:
    for text in rows:
        b.write(json.dumps({"prompt": text}) + "\n")
        # output_length makes AIPerf send max_completion_tokens and track OSL
        # mismatch, as in the epic #184 run.
        a.write(json.dumps({"text": text, "output_length": max_tokens}) + "\n")
print(f"prompts: {len(rows)} rows", file=sys.stderr)
PY
}

# parity_sut OUT BIN -> writes OUT/sut.json describing the mock and the binary.
# No cost block: cost is a scenario input (--price-per-hour), not a SUT default.
parity_sut() {
  local out="$1" bin="$2" ident
  ident="$(parity_bench_identity "${bin}")"
  # Values travel through argv, never pasted into Python source.
  "${PYTHON}" - "${out}/sut.json" "${ident}" "${MODEL}" "${PREFILL_MS}" \
    "${PER_PROMPT_TOKEN_MS}" "${ITL_MS}" "${MOCK_SEED}" <<'PY'
import json, platform, sys
path, ident, model = sys.argv[1:4]
prefill, per_tok, itl = (float(v) for v in sys.argv[4:7])
seed = int(sys.argv[7])
json.dump({
  "provenance": "declared",
  "name": "parity-mock (scripts/parity/mock_server.py, no GPU)",
  "vendor": "Metrum AI parity harness",
  "gpu": {"model": "none (CPU mock server)", "count": 1},
  "driver_version": "n/a (mock)",
  "runtime": {"name": "metrum-parity-mock", "version": "scripts/parity",
              "config": f"mock_server.py --prefill-ms {prefill:g} --per-prompt-token-ms "
                        f"{per_tok:g} --itl-ms {itl:g} --seed {seed}"},
  "model": {"id": model},
  "host_os": platform.platform(),
  "notes": "Data-point count run (#204), not a performance result. Bench " + ident,
}, open(path, "w"), indent=2)
PY
}
