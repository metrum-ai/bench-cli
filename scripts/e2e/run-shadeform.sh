#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Shadeform telemetry e2e: RTXPro6000 > H200 > H100, Qwen/Qwen3.8-27B,
# Jarvis-style concurrency sweep (1..64), all-smi fork /metric.
# Hard cap: 3 GPU-hours. Secrets from env only.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SHADEFORM="${REPO_ROOT}/scripts/live/shadeform.sh"
ART="${REPO_ROOT}/artifacts/e2e"
RESULTS_REMOTE="/tmp/metrum-e2e"
MODEL="${MODEL:-Qwen/Qwen3.8-27B}"
BUDGET_HOURS="${BUDGET_HOURS:-3}"
HOURLY_CENTS=""
INSTANCE_ID=""
INSTANCE_IP=""
START_EPOCH="$(date -u +%s)"

mkdir -p "${ART}"
chmod +x "${REPO_ROOT}/scripts/e2e/sut-setup.sh"

log() { echo "[e2e $(date -u +%Y-%m-%dT%H:%M:%SZ)] $*" | tee -a "${ART}/cost.txt"; }

cost_tick() {
  local now elapsed hours est
  now="$(date -u +%s)"
  elapsed=$((now - START_EPOCH))
  hours="$(awk -v s="${elapsed}" 'BEGIN{printf "%.3f", s/3600}')"
  if [[ -n "${HOURLY_CENTS}" ]]; then
    est="$(awk -v h="${hours}" -v c="${HOURLY_CENTS}" 'BEGIN{printf "%.2f", h*(c/100)}')"
    log "cost estimate: elapsed_h=${hours} hourly=\$$(awk -v c="${HOURLY_CENTS}" 'BEGIN{printf "%.2f", c/100}') est_usd=${est}"
  else
    log "elapsed_h=${hours} (hourly unknown)"
  fi
  awk -v h="${hours}" -v b="${BUDGET_HOURS}" 'BEGIN{exit !(h>=b)}' && {
    log "budget ${BUDGET_HOURS}h exceeded; tearing down"
    exit 99
  }
}

cleanup() {
  local rc=$?
  log "cleanup rc=${rc}"
  if [[ -n "${INSTANCE_ID}" ]]; then
    "${SHADEFORM}" delete "${INSTANCE_ID}" || true
    local gone=0
    for _ in $(seq 1 36); do
      # Prefer a cheap info probe; do not block on shadeform wait (hangs on deleted).
      if ! curl -fsS --max-time 5 -H "X-API-KEY: ${SHADEFORM_API_KEY}" \
        "https://api.shadeform.ai/v1/instances/${INSTANCE_ID}/info" >/dev/null 2>&1; then
        gone=1
        break
      fi
      sleep 5
    done
    log "deletion_timestamp=$(date -u +%Y-%m-%dT%H:%M:%SZ) gone=${gone}"
    curl -fsS -H "X-API-KEY: ${SHADEFORM_API_KEY}" \
      "https://api.shadeform.ai/v1/instances" | jq '[.instances[]? | {id,name,status,shade_instance_type}]' \
      >"${ART}/instances-after.json" || true
  fi
  cost_tick || true
}
trap cleanup EXIT

: "${SHADEFORM_API_KEY:?set SHADEFORM_API_KEY}"
# Public Hub dataset; token optional (rate limits / private mirrors).
HF_TOKEN="${HF_TOKEN:-}"

# Fail fast if the API key cannot manage instances (types is public).
INST_CODE="$(curl -sS -o /tmp/sf-instances.json -w '%{http_code}' \
  -H "X-API-KEY: ${SHADEFORM_API_KEY}" -H "Accept: application/json" \
  https://api.shadeform.ai/v1/instances || true)"
if [[ "${INST_CODE}" != "200" ]]; then
  log "Shadeform /v1/instances returned HTTP ${INST_CODE}; refresh SHADEFORM_API_KEY"
  exit 1
fi

# SSH key: prefer SHADEFORM_SSH_KEY_ID. Else upload/match local pubkey
# (${SHADEFORM_SSH_IDENTITY:-~/.ssh/id_ed25519}.pub) so create + ssh use the same key.
SSH_IDENTITY="${SHADEFORM_SSH_IDENTITY:-${HOME}/.ssh/id_ed25519}"
export SSH_IDENTITY
SSH_KEY_ID="${SHADEFORM_SSH_KEY_ID:-}"
if [[ -z "${SSH_KEY_ID}" && -f "${SSH_IDENTITY}.pub" ]]; then
  SSH_KEY_ID="$(python3 - <<'PY'
import json, os, pathlib, urllib.request
api = os.environ["SHADEFORM_API_KEY"]
pub = pathlib.Path(os.environ["SSH_IDENTITY"] + ".pub").read_text().strip()
parts = pub.split()
local = " ".join(parts[:2]) if len(parts) >= 2 else pub

def body(s: str) -> str:
    p = s.split()
    return " ".join(p[:2]) if len(p) >= 2 else s

req = urllib.request.Request(
    "https://api.shadeform.ai/v1/sshkeys",
    headers={"X-API-KEY": api, "Accept": "application/json"},
)
keys = json.load(urllib.request.urlopen(req)).get("ssh_keys", [])
for k in keys:
    if body(k.get("public_key") or "") == local:
        print(k["id"])
        raise SystemExit(0)
payload = json.dumps({"name": "metrum-e2e-local", "public_key": pub}).encode()
req = urllib.request.Request(
    "https://api.shadeform.ai/v1/sshkeys/add",
    data=payload,
    method="POST",
    headers={
        "X-API-KEY": api,
        "Content-Type": "application/json",
        "Accept": "application/json",
    },
)
print(json.load(urllib.request.urlopen(req))["id"])
PY
)"
  export SSH_IDENTITY
fi
if [[ -z "${SSH_KEY_ID}" ]]; then
  if SSH_JSON="$(curl -fsS -H "X-API-KEY: ${SHADEFORM_API_KEY}" \
    https://api.shadeform.ai/v1/sshkeys 2>/dev/null)"; then
    SSH_KEY_ID="$(jq -r '.ssh_keys[] | select(.is_default==true) | .id' <<<"${SSH_JSON}" | head -1)"
    if [[ -z "${SSH_KEY_ID}" || "${SSH_KEY_ID}" == "null" ]]; then
      SSH_KEY_ID="$(jq -r '.ssh_keys[0].id // empty' <<<"${SSH_JSON}")"
    fi
  fi
fi
: "${SSH_KEY_ID:?set SHADEFORM_SSH_KEY_ID or provide ${SSH_IDENTITY}.pub}"
export SHADEFORM_SSH_KEY_ID="${SSH_KEY_ID}"
log "using ssh_key_id=${SSH_KEY_ID} identity=${SSH_IDENTITY}"


# Prefer RTXPro6000 (confirmed available), then H200, then H100 single-GPU.
pick_json=""
for want in RTXPro6000 H200 H100; do
  pick_json="$(curl -fsS -H "X-API-KEY: ${SHADEFORM_API_KEY}" \
    https://api.shadeform.ai/v1/instances/types | jq -c --arg g "${want}" '
      [.instance_types[]
        | select(.gpu_type==$g)
        | . as $it
        | ($it.availability // [])[]
        | select(.available==true)
        | {cloud:$it.cloud, shade_instance_type:$it.shade_instance_type, region:.region,
           gpu_type:$it.gpu_type, num_gpus:($it.num_gpus//1), hourly_price:($it.hourly_price//0)}
        | select(.num_gpus==1)
      ] | sort_by(.hourly_price) | .[0] // empty')"
  if [[ -n "${pick_json}" && "${pick_json}" != "null" && "${pick_json}" != "" ]]; then
    log "picked ${want}: ${pick_json}"
    break
  fi
  log "no single-GPU ${want} available; trying next"
done
[[ -n "${pick_json}" && "${pick_json}" != "null" && "${pick_json}" != "" ]] \
  || { log "no RTXPro6000/H200/H100x1 available"; exit 1; }

HOURLY_CENTS="$(jq -r '.hourly_price' <<<"${pick_json}")"
CLOUD="$(jq -r '.cloud' <<<"${pick_json}")"
REGION="$(jq -r '.region' <<<"${pick_json}")"
TYPE="$(jq -r '.shade_instance_type' <<<"${pick_json}")"

# Create bare instance (we install exporters ourselves; no public docker launch for vLLM)
NAME="metrum-telemetry-$(date -u +%Y%m%d-%H%M%S)"
CREATE_PAYLOAD="$(jq -n \
  --arg cloud "${CLOUD}" --arg region "${REGION}" --arg type "${TYPE}" \
  --arg name "${NAME}" --arg ssh "${SSH_KEY_ID:-}" \
  '{cloud:$cloud, region:$region, shade_instance_type:$type, shade_cloud:true, name:$name}
   | if $ssh == "" then . else . + {ssh_key_id:$ssh} end')"
log "creating instance"
CREATE_RESP="$(curl -fsS -X POST -H "X-API-KEY: ${SHADEFORM_API_KEY}" \
  -H "Content-Type: application/json" \
  -d "${CREATE_PAYLOAD}" \
  https://api.shadeform.ai/v1/instances/create)"
INSTANCE_ID="$(jq -r '.id // .instance_id' <<<"${CREATE_RESP}")"
log "instance_id=${INSTANCE_ID}"
jq 'del(.ssh_private_key?, .password?) | .ip="REDACTED" | .hostname="REDACTED"' \
  <<<"${CREATE_RESP}" >"${ART}/instance-create.json" || echo "${CREATE_RESP}" >"${ART}/instance-create.json"

INSTANCE_IP="$("${SHADEFORM}" wait "${INSTANCE_ID}")"
log "instance active ip=REDACTED"

# Redacted instance info
curl -fsS -H "X-API-KEY: ${SHADEFORM_API_KEY}" \
  "https://api.shadeform.ai/v1/instances/${INSTANCE_ID}/info" \
  | jq 'del(.ssh_private_key?, .password?) | .ip="REDACTED" | (.configuration.ssh?)=null | .hostname="REDACTED"' \
  >"${ART}/instance.json"

SSH=(
  ssh
  -i "${SSH_IDENTITY}"
  -o IdentitiesOnly=yes
  -o StrictHostKeyChecking=no
  -o UserKnownHostsFile=/dev/null
  -o ConnectTimeout=20
)
SCP=(
  scp
  -i "${SSH_IDENTITY}"
  -o IdentitiesOnly=yes
  -o StrictHostKeyChecking=no
  -o UserKnownHostsFile=/dev/null
)
# Discover user
SSH_USER="$(curl -fsS -H "X-API-KEY: ${SHADEFORM_API_KEY}" \
  "https://api.shadeform.ai/v1/instances/${INSTANCE_ID}/info" | jq -r '.ssh_user // .configuration.ssh_user // "ubuntu"')"
REMOTE="${SSH_USER}@${INSTANCE_IP}"

cost_loop() {
  while true; do
    sleep 600
    cost_tick || exit 99
  done
}
cost_loop &
COST_PID=$!

remote() { "${SSH[@]}" "${REMOTE}" "$@"; }
remote_sudo() { "${SSH[@]}" "${REMOTE}" "sudo bash -lc $(printf '%q' "$*")"; }

log "waiting for ssh"
ssh_ok=0
for _ in $(seq 1 60); do
  if "${SSH[@]}" "${REMOTE}" 'echo ok' >/dev/null 2>&1; then
    ssh_ok=1
    break
  fi
  sleep 5
done
[[ "${ssh_ok}" -eq 1 ]] || { log "ssh failed for ${REMOTE} with identity ${SSH_IDENTITY}"; exit 1; }

log "installing docker/nvidia toolkit if needed"
remote 'command -v docker >/dev/null || (curl -fsSL https://get.docker.com | sudo sh)'
remote 'sudo usermod -aG docker "$USER" || true'
remote 'command -v nvidia-smi'

# Sync repo binaries: build release locally and scp, or build on remote.
BIN_DIR="${CARGO_TARGET_DIR:-${REPO_ROOT}/target}/release"
log "building release binaries locally (BIN_DIR=${BIN_DIR})"
(cd "${REPO_ROOT}" && cargo build --release --bin metrum-ai-bench-cli-strategic --bin metrum-ai-bench-cli-prompts --bin metrum-ai-bench-cli-mock-server)

log "copying tools to remote"
remote "mkdir -p ${RESULTS_REMOTE}/bin ${RESULTS_REMOTE}/prompts ${RESULTS_REMOTE}/promptfoo"
"${SCP[@]}" \
  "${BIN_DIR}/metrum-ai-bench-cli-strategic" \
  "${BIN_DIR}/metrum-ai-bench-cli-prompts" \
  "${REPO_ROOT}/scripts/e2e/sut-setup.sh" \
  "${REPO_ROOT}/scripts/e2e/interrupt-run.sh" \
  "${REPO_ROOT}/scripts/e2e/aiperf-bakeoff.sh" \
  "${REMOTE}:${RESULTS_REMOTE}/bin/"
"${SCP[@]}" -r \
  "${REPO_ROOT}/scripts/e2e/promptfoo/." \
  "${REMOTE}:${RESULTS_REMOTE}/promptfoo/"
remote "chmod +x ${RESULTS_REMOTE}/bin/*"

log "running sut-setup"
remote "export HF_TOKEN=$(printf '%q' "${HF_TOKEN}"); export MODEL=$(printf '%q' "${MODEL}"); ${RESULTS_REMOTE}/bin/sut-setup.sh"

# Prompts: required Hub mix from https://huggingface.co/datasets/metrum-ai/prompt-library
# Default: latest `main` (resolved SHA recorded in mix-report). Override with
# PROMPT_LIBRARY_REVISION=<40-char sha> to pin. No synthetic fallback.
PROMPT_LIBRARY_DATASET="${PROMPT_LIBRARY_DATASET:-metrum-ai/prompt-library}"
PROMPT_LIBRARY_REVISION="${PROMPT_LIBRARY_REVISION:-main}"
PROMPT_LIBRARY_CONFIG="${PROMPT_LIBRARY_CONFIG:-sample}"
PROMPT_LIBRARY_PROFILE="${PROMPT_LIBRARY_PROFILE:-rag-medium}"
log "extracting prompts from https://huggingface.co/datasets/${PROMPT_LIBRARY_DATASET} revision=${PROMPT_LIBRARY_REVISION} config=${PROMPT_LIBRARY_CONFIG} profile=${PROMPT_LIBRARY_PROFILE}"
remote "export HF_TOKEN=$(printf '%q' "${HF_TOKEN}"); \
  ${RESULTS_REMOTE}/bin/metrum-ai-bench-cli-prompts \
  --dataset ${PROMPT_LIBRARY_DATASET} \
  --revision ${PROMPT_LIBRARY_REVISION} \
  --config ${PROMPT_LIBRARY_CONFIG} \
  --profile ${PROMPT_LIBRARY_PROFILE} \
  --count 48 \
  --output ${RESULTS_REMOTE}/prompts/mix.jsonl \
  --report ${RESULTS_REMOTE}/prompts/mix-report.json"
remote "python3 - <<'PY'
import json, sys, re
report=json.load(open('${RESULTS_REMOTE}/prompts/mix-report.json',encoding='utf-8'))
ds=str(report.get('dataset') or '')
rev=str(report.get('revision') or '')
n=report.get('selected_count')
want='${PROMPT_LIBRARY_REVISION}'
print('prompt-library report dataset=%s revision=%s config=%s profile=%s selected=%s' % (
  ds, rev, report.get('config'), report.get('profile'), n))
if ds != 'metrum-ai/prompt-library':
  sys.exit('expected dataset metrum-ai/prompt-library, got %r' % ds)
if not re.fullmatch(r'[0-9a-f]{40}', rev):
  sys.exit('expected resolved 40-char sha in mix-report, got %r' % rev)
if re.fullmatch(r'[0-9a-f]{40}', want) and rev != want:
  sys.exit('expected pinned revision %s, got %r' % (want, rev))
mix=open('${RESULTS_REMOTE}/prompts/mix.jsonl',encoding='utf-8').read().strip().splitlines()
if len(mix) < 8:
  sys.exit('prompt-library mix too small: %d rows' % len(mix))
print('prompt-library mix ok rows=%d resolved_revision=%s' % (len(mix), rev))
PY"

# Jarvis-style closed-loop sweep: 1,2,4,8,16,32,64 concurrency; streaming
log "closed-loop concurrency sweep"
METRUM_CLOSED_START="$(date -u +%s)"
remote "cd ${RESULTS_REMOTE} && ./bin/metrum-ai-bench-cli-strategic \
  --url http://127.0.0.1:8000/v1/chat/completions \
  --api-key dummy \
  --model sut \
  --streaming \
  --prompts ${RESULTS_REMOTE}/prompts/mix.jsonl \
  --max-tokens 256 \
  --ignore-eos \
  --warmup-requests 4 \
  --requests-per-stage 16 \
  --sweep 1,2,4,8,16,32,64 \
  --sweep-by concurrency \
  --sut ${RESULTS_REMOTE}/sut.json --require-sut \
  --telemetry ${RESULTS_REMOTE}/telemetry.yaml \
  --require-telemetry \
  --ndjson ${RESULTS_REMOTE}/run-closed.ndjson \
  --html ${RESULTS_REMOTE}/report-closed.html \
  --csv ${RESULTS_REMOTE}/requests-closed.csv \
  > ${RESULTS_REMOTE}/stdout-closed.json"
remote "python3 - <<'PY'
import json, sys
path='${RESULTS_REMOTE}/run-closed.ndjson'
reqs=succ=0
with open(path,encoding='utf-8') as f:
  for line in f:
    row=json.loads(line)
    if row.get('kind')!='request':
      continue
    if row.get('warmup'):
      continue
    reqs += 1
    if row.get('success') is True:
      succ += 1
print(f'closed-loop measured={reqs} success={succ}')
if reqs < 8:
  sys.exit('closed-loop produced too few measured requests')
if succ < max(4, reqs // 10):
  sys.exit(f'closed-loop success rate too low: {succ}/{reqs}')
PY"
METRUM_CLOSED_END="$(date -u +%s)"
remote "python3 - <<PY
import json, time
timings = {
  'tool': 'metrum-ai-bench-cli-strategic',
  'phase': 'closed-loop',
  'setup_seconds': None,
  'run_seconds': ${METRUM_CLOSED_END} - ${METRUM_CLOSED_START},
  'total_seconds': ${METRUM_CLOSED_END} - ${METRUM_CLOSED_START},
  'notes': 'closed-loop wall time only (excludes cargo/scp/sut-setup); see cost.txt for full driver elapsed',
  'finished_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
}
open('${RESULTS_REMOTE}/metrum_timings.json','w',encoding='utf-8').write(json.dumps(timings, indent=2)+'\\n')
print(json.dumps(timings))
PY"

# Open-loop lighter sweep
log "open-loop rate sweep"
remote "cd ${RESULTS_REMOTE} && ./bin/metrum-ai-bench-cli-strategic \
  --url http://127.0.0.1:8000/v1/chat/completions \
  --api-key dummy \
  --model sut \
  --streaming \
  --prompts ${RESULTS_REMOTE}/prompts/mix.jsonl \
  --max-tokens 256 \
  --ignore-eos \
  --warmup-requests 4 \
  --requests-per-stage 16 \
  --sweep 2,4,8,16 \
  --sweep-by rate \
  --max-in-flight 32 \
  --sut ${RESULTS_REMOTE}/sut.json --require-sut \
  --telemetry ${RESULTS_REMOTE}/telemetry.yaml \
  --require-telemetry \
  --ndjson ${RESULTS_REMOTE}/run-open.ndjson \
  --html ${RESULTS_REMOTE}/report-open.html \
  --csv ${RESULTS_REMOTE}/requests-open.csv \
  > ${RESULTS_REMOTE}/stdout-open.json"

# Interrupt: best-effort SIGINT partial summary (do not block promptfoo/AIPerf).
log "interrupt partial-summary check"
set +e
remote "chmod +x ${RESULTS_REMOTE}/bin/interrupt-run.sh && ${RESULTS_REMOTE}/bin/interrupt-run.sh"
INTERRUPT_RC=$?
set -e
if [[ "${INTERRUPT_RC}" -ne 0 ]]; then
  log "warning: interrupt check failed rc=${INTERRUPT_RC}; continuing to promptfoo/AIPerf"
fi

# Promptfoo: fast smoke suites with a hard ~30 minute wall-clock budget (both suites).
PROMPTFOO_BUDGET_SEC="${PROMPTFOO_BUDGET_SEC:-1800}"
log "installing promptfoo (budget starts after install)"
remote 'command -v node >/dev/null || (curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash - && sudo apt-get install -y nodejs)'
remote 'command -v promptfoo >/dev/null || sudo npm install -g promptfoo'

log "promptfoo general+coding (wall budget ${PROMPTFOO_BUDGET_SEC}s)"
remote "cd ${RESULTS_REMOTE}/promptfoo && \
  export OPENAI_API_KEY=dummy OPENAI_BASE_URL=http://127.0.0.1:8000/v1 && \
  export PROMPTFOO_DISABLE_TELEMETRY=1 && \
  rm -f ${RESULTS_REMOTE}/promptfoo-general.json ${RESULTS_REMOTE}/promptfoo-coding.json \
        ${RESULTS_REMOTE}/promptfoo-general.txt ${RESULTS_REMOTE}/promptfoo-coding.txt && \
  /usr/bin/timeout -k 30 ${PROMPTFOO_BUDGET_SEC} bash -lc '
    set -e
    promptfoo eval -c general.yaml --no-cache -o ${RESULTS_REMOTE}/promptfoo-general.json \
      | tee ${RESULTS_REMOTE}/promptfoo-general.txt
    promptfoo eval -c coding.yaml --no-cache -o ${RESULTS_REMOTE}/promptfoo-coding.json \
      | tee ${RESULTS_REMOTE}/promptfoo-coding.txt
  '"
PROMPTFOO_RC=$?
if [[ "${PROMPTFOO_RC}" -eq 124 ]] || [[ "${PROMPTFOO_RC}" -eq 137 ]]; then
  log "error: promptfoo exceeded ${PROMPTFOO_BUDGET_SEC}s budget"
  exit 1
fi
if [[ "${PROMPTFOO_RC}" -ne 0 ]]; then
  log "error: promptfoo failed rc=${PROMPTFOO_RC}"
  exit 1
fi

# Summarize pass rates; require successful non-empty suites.
remote "python3 - <<'PY'
import json, pathlib, sys
root = pathlib.Path('${RESULTS_REMOTE}')
out = {}
errors = []
for name in ('general', 'coding'):
    p = root / f'promptfoo-{name}.json'
    if not p.exists():
        out[name] = {'error': 'missing'}
        errors.append(name + ': missing output')
        continue
    data = json.loads(p.read_text())
    results = (data.get('results') or {}).get('results') or data.get('results') or []
    if isinstance(results, dict):
        results = results.get('results') or []
    n = len(results)
    passed = sum(1 for r in results if (r.get('success') is True) or (r.get('score') or 0) >= 1)
    out[name] = {'cases': n, 'passed': passed, 'pass_rate': (passed / n if n else 0.0)}
    if n < 1:
        errors.append(name + ': zero cases')
    if passed < 1:
        errors.append(name + ': zero passes')
(root / 'promptfoo-summary.json').write_text(json.dumps(out, indent=2) + '\n')
print(json.dumps(out))
if errors:
    sys.exit('promptfoo success gate failed: ' + '; '.join(errors))
PY"

# NVIDIA AIPerf bake-off on the same live SUT + same Hub prompt mix.
log "AIPerf bake-off (same SUT, same prompt-library mix)"
remote "chmod +x ${RESULTS_REMOTE}/bin/aiperf-bakeoff.sh && ${RESULTS_REMOTE}/bin/aiperf-bakeoff.sh"
remote "python3 - <<'PY'
import json, sys
from pathlib import Path
t = json.loads(Path('${RESULTS_REMOTE}/aiperf/timings.json').read_text())
stages = t.get('stages') or []
ok = [s for s in stages if s.get('rc') == 0]
print('aiperf stages=%d ok=%d setup_s=%s run_s=%s' % (
  len(stages), len(ok), t.get('setup_seconds'), t.get('run_seconds')))
if len(ok) < 3:
  sys.exit('aiperf success gate failed: need >=3 successful concurrency stages')
PY"

# Analyze locally after pull
log "fetching artifacts"
mkdir -p "${ART}/raw"
"${SCP[@]}" -r \
  "${REMOTE}:${RESULTS_REMOTE}/run-closed.ndjson" \
  "${REMOTE}:${RESULTS_REMOTE}/run-open.ndjson" \
  "${REMOTE}:${RESULTS_REMOTE}/run-interrupt.ndjson" \
  "${REMOTE}:${RESULTS_REMOTE}/report-closed.html" \
  "${REMOTE}:${RESULTS_REMOTE}/report-open.html" \
  "${REMOTE}:${RESULTS_REMOTE}/stdout-closed.json" \
  "${REMOTE}:${RESULTS_REMOTE}/stdout-open.json" \
  "${REMOTE}:${RESULTS_REMOTE}/stdout-interrupt.json" \
  "${REMOTE}:${RESULTS_REMOTE}/sut.json" \
  "${REMOTE}:${RESULTS_REMOTE}/telemetry.yaml" \
  "${REMOTE}:${RESULTS_REMOTE}/prompts/mix-report.json" \
  "${REMOTE}:${RESULTS_REMOTE}/promptfoo-general.json" \
  "${REMOTE}:${RESULTS_REMOTE}/promptfoo-general.txt" \
  "${REMOTE}:${RESULTS_REMOTE}/promptfoo-coding.json" \
  "${REMOTE}:${RESULTS_REMOTE}/promptfoo-coding.txt" \
  "${REMOTE}:${RESULTS_REMOTE}/promptfoo-summary.json" \
  "${REMOTE}:${RESULTS_REMOTE}/metrum_timings.json" \
  "${ART}/raw/" || true

# AIPerf bake-off tree (may be large; pull timings + stage summaries + exports)
mkdir -p "${ART}/raw/aiperf" "${ART}/aiperf"
"${SCP[@]}" -r \
  "${REMOTE}:${RESULTS_REMOTE}/aiperf/timings.json" \
  "${REMOTE}:${RESULTS_REMOTE}/aiperf/stages.jsonl" \
  "${REMOTE}:${RESULTS_REMOTE}/aiperf/aiperf-input.jsonl" \
  "${ART}/raw/aiperf/" || true
# Per-concurrency exports (best effort)
for c in 1 2 4 8 16 32 64; do
  "${SCP[@]}" -r "${REMOTE}:${RESULTS_REMOTE}/aiperf/c${c}" "${ART}/raw/aiperf/" 2>/dev/null || true
done
cp -a "${ART}/raw/aiperf/." "${ART}/aiperf/" 2>/dev/null || true
cp "${ART}/raw/metrum_timings.json" "${ART}/" 2>/dev/null || true
cp "${ART}/raw/mix-report.json" "${ART}/" 2>/dev/null || true

# Compress ndjson
if command -v zstd >/dev/null; then
  zstd -f -19 -o "${ART}/run-closed.ndjson.zst" "${ART}/raw/run-closed.ndjson"
  zstd -f -19 -o "${ART}/run-open.ndjson.zst" "${ART}/raw/run-open.ndjson"
else
  gzip -c "${ART}/raw/run-closed.ndjson" >"${ART}/run-closed.ndjson.gz"
  gzip -c "${ART}/raw/run-open.ndjson" >"${ART}/run-open.ndjson.gz"
fi
cp "${ART}/raw/report-closed.html" "${ART}/" 2>/dev/null || true
cp "${ART}/raw/report-open.html" "${ART}/" 2>/dev/null || true
cp "${ART}/raw/sut.json" "${ART}/"
cp "${ART}/raw/telemetry.yaml" "${ART}/"
cp "${ART}/raw/stdout-closed.json" "${ART}/"
cp "${ART}/raw/stdout-open.json" "${ART}/"
cp "${ART}/raw/promptfoo-general.json" "${ART}/" 2>/dev/null || true
cp "${ART}/raw/promptfoo-general.txt" "${ART}/" 2>/dev/null || true
cp "${ART}/raw/promptfoo-coding.json" "${ART}/" 2>/dev/null || true
cp "${ART}/raw/promptfoo-coding.txt" "${ART}/" 2>/dev/null || true
cp "${ART}/raw/promptfoo-summary.json" "${ART}/" 2>/dev/null || true

# Validation via analyze.py if present
if [[ -f "${REPO_ROOT}/docs/queries/analyze.py" ]]; then
  python3 "${REPO_ROOT}/docs/queries/analyze.py" "${ART}/raw/run-closed.ndjson" \
    | tee "${ART}/analyze-closed.txt" || true
  python3 "${REPO_ROOT}/docs/queries/analyze.py" "${ART}/raw/run-open.ndjson" \
    | tee "${ART}/analyze-open.txt" || true
fi

# Metrum vs AIPerf comparison report (dataset + methodology + timings + metrics)
python3 "${REPO_ROOT}/scripts/e2e/write_aiperf_comparison.py" --art "${ART}" \
  || log "warning: AIPerf comparison report generation failed"

cat >"${ART}/VALIDATION.md" <<EOF
<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Shadeform telemetry validation

- timestamp_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)
- model: ${MODEL}
- instance_type: ${TYPE}
- region: ${REGION}
- cloud: ${CLOUD}
- dataset: https://huggingface.co/datasets/metrum-ai/prompt-library (pinned revision; see mix-report.json)
- workload: Hub \`rag-medium\` mix, closed concurrency 1..64, open rate sweep, SIGINT partial, promptfoo general+coding, AIPerf bake-off
- validation: closed sweep, open rate sweep, SIGINT partial summary, promptfoo general + coding, AIPerf comparison
- telemetry: all-smi (Metrum fork /metric), vllm, node, optional dcgm/cadvisor
- artifacts: run-closed/open/interrupt ndjson (compressed), HTML reports, sut.json, telemetry.yaml, stdout JSON, promptfoo-*.json/txt, aiperf/, COMPARISON_AIPERF.md

See \`COMPARISON_AIPERF.md\` for metrum vs NVIDIA AIPerf methodology, setup/run timings, and metric comparison.
See analyze-closed.txt / analyze-open.txt, promptfoo-summary.json, and README_BUNDLE.md for offline analysis.
EOF

cat >"${ART}/README_BUNDLE.md" <<'EOF'
<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Telemetry e2e report bundle

Use this directory in a separate agent session.

## Files
- `run-closed.ndjson.zst` / `run-open.ndjson.zst`: tagged telemetry NDJSON
- `report-*.html`: knee / throughput HTML
- `stdout-*.json`: strategic summary JSON (`points`, knee)
- `sut.json`, `telemetry.yaml`
- `mix-report.json`: Hub prompt-library selection report
- `promptfoo-general.json` / `promptfoo-coding.json`: full promptfoo eval outputs
- `promptfoo-general.txt` / `promptfoo-coding.txt`: console tables
- `promptfoo-summary.json`: pass rates for general + coding
- `aiperf/`: NVIDIA AIPerf bake-off artifacts + `timings.json`
- `COMPARISON_AIPERF.md`: metrum vs AIPerf study (dataset, methodology, timings, metrics, observations)
- `metrum_timings.json`: closed-loop wall clock for bake-off
- `VALIDATION.md`, `cost.txt`, `analyze-*.txt`
- `instance.json`: redacted Shadeform instance metadata

## Offline analysis
1. Decompress: `zstd -d run-closed.ndjson.zst`
2. Read `docs/TELEMETRY.md` and `docs/telemetry/ANALYSIS.md`
3. Run `python3 docs/queries/analyze.py run-closed.ndjson`
4. Or DuckDB: `duckdb -c ".read docs/queries/stage_power.sql"` after setting the input path
5. Inspect promptfoo: `jq . promptfoo-summary.json` and the per-suite JSON
6. Read `COMPARISON_AIPERF.md` for the AIPerf bake-off

Do not invent metric names; use the include list in `telemetry.yaml`.
EOF

log "bundle ready under ${ART}"
kill "${COST_PID}" 2>/dev/null || true
cost_tick || true
