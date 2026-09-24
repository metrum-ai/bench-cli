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
    for _ in $(seq 1 60); do
      if ! "${SHADEFORM}" wait "${INSTANCE_ID}" >/dev/null 2>&1; then
        # wait dies on deleted; also try info
        if ! curl -fsS -H "X-API-KEY: ${SHADEFORM_API_KEY}" \
          "https://api.shadeform.ai/v1/instances/${INSTANCE_ID}/info" >/dev/null 2>&1; then
          gone=1
          break
        fi
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
: "${HF_TOKEN:?set HF_TOKEN}"

# Fail fast if the API key cannot manage instances (types is public).
INST_CODE="$(curl -sS -o /tmp/sf-instances.json -w '%{http_code}' \
  -H "X-API-KEY: ${SHADEFORM_API_KEY}" -H "Accept: application/json" \
  https://api.shadeform.ai/v1/instances || true)"
if [[ "${INST_CODE}" != "200" ]]; then
  log "Shadeform /v1/instances returned HTTP ${INST_CODE}; refresh SHADEFORM_API_KEY"
  exit 1
fi

# SSH key: prefer SHADEFORM_SSH_KEY_ID, else account default, else omit (managed key).
SSH_KEY_ID="${SHADEFORM_SSH_KEY_ID:-}"
if [[ -z "${SSH_KEY_ID}" ]]; then
  if SSH_JSON="$(curl -fsS -H "X-API-KEY: ${SHADEFORM_API_KEY}" \
    https://api.shadeform.ai/v1/sshkeys 2>/dev/null)"; then
    SSH_KEY_ID="$(jq -r '.ssh_keys[] | select(.is_default==true) | .id' <<<"${SSH_JSON}" | head -1)"
    if [[ -z "${SSH_KEY_ID}" || "${SSH_KEY_ID}" == "null" ]]; then
      SSH_KEY_ID="$(jq -r '.ssh_keys[0].id // empty' <<<"${SSH_JSON}")"
    fi
  fi
fi
if [[ -n "${SSH_KEY_ID}" ]]; then
  export SHADEFORM_SSH_KEY_ID="${SSH_KEY_ID}"
  log "using ssh_key_id=${SSH_KEY_ID}"
else
  log "no ssh_key_id; create will use Shadeform managed key"
fi

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

SSH=(ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=20)
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
for _ in $(seq 1 60); do
  if "${SSH[@]}" "${REMOTE}" 'echo ok' >/dev/null 2>&1; then break; fi
  sleep 5
done

log "installing docker/nvidia toolkit if needed"
remote 'command -v docker >/dev/null || (curl -fsSL https://get.docker.com | sudo sh)'
remote 'sudo usermod -aG docker "$USER" || true'
remote 'command -v nvidia-smi'

# Sync repo binaries: build release locally and scp, or build on remote.
log "building release binaries locally"
(cd "${REPO_ROOT}" && cargo build --release --bin metrum-ai-bench-cli-strategic --bin metrum-ai-bench-cli-prompts --bin metrum-ai-bench-cli-mock-server)

log "copying tools to remote"
remote "mkdir -p ${RESULTS_REMOTE}/bin ${RESULTS_REMOTE}/prompts"
scp -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  "${REPO_ROOT}/target/release/metrum-ai-bench-cli-strategic" \
  "${REPO_ROOT}/target/release/metrum-ai-bench-cli-prompts" \
  "${REPO_ROOT}/scripts/e2e/sut-setup.sh" \
  "${REMOTE}:${RESULTS_REMOTE}/bin/"
remote "chmod +x ${RESULTS_REMOTE}/bin/*"

log "running sut-setup"
remote "export HF_TOKEN=$(printf '%q' "${HF_TOKEN}"); export MODEL=$(printf '%q' "${MODEL}"); ${RESULTS_REMOTE}/bin/sut-setup.sh"

# Prompts: coding-style long context (~16k input tokens target via prompt library if possible)
log "extracting prompts"
remote "${RESULTS_REMOTE}/bin/metrum-ai-bench-cli-prompts \
  --dataset metrum-ai/prompt-library \
  --config metrum-ai-bench-cli-prompts \
  --isl-target 16000 --osl-target 512 --count 64 \
  --out ${RESULTS_REMOTE}/prompts/mix.jsonl \
  --report ${RESULTS_REMOTE}/prompts/mix-report.json" \
  || remote "python3 - <<'PY'
import json
path='${RESULTS_REMOTE}/prompts/mix.jsonl'
# Fallback: synthetic long prompts (~16k tokens ~ 64k chars)
pad='def foo():\\n    return 1\\n' * 2000
with open(path,'w') as f:
  for i in range(64):
    f.write(json.dumps({'prompt': f'Refactor this module and explain changes. id={i}\\n'+pad[:60000]})+'\\n')
print('wrote fallback prompts', path)
PY"

# Jarvis-style closed-loop sweep: 1,2,4,8,16,32,64 concurrency; 512 out; streaming
log "closed-loop concurrency sweep"
remote "cd ${RESULTS_REMOTE} && ./bin/metrum-ai-bench-cli-strategic \
  --url http://127.0.0.1:8000/v1/chat/completions \
  --api-key dummy \
  --model sut \
  --streaming \
  --prompts ${RESULTS_REMOTE}/prompts/mix.jsonl \
  --max-tokens 512 \
  --ignore-eos \
  --warmup-requests 8 \
  --requests-per-stage 32 \
  --sweep 1,2,4,8,16,32,64 \
  --sweep-by concurrency \
  --sut ${RESULTS_REMOTE}/sut.json --require-sut \
  --telemetry ${RESULTS_REMOTE}/telemetry.yaml \
  --require-telemetry \
  --ndjson ${RESULTS_REMOTE}/run-closed.ndjson \
  --html ${RESULTS_REMOTE}/report-closed.html \
  --csv ${RESULTS_REMOTE}/requests-closed.csv \
  > ${RESULTS_REMOTE}/stdout-closed.json"

# Open-loop lighter sweep
log "open-loop rate sweep"
remote "cd ${RESULTS_REMOTE} && ./bin/metrum-ai-bench-cli-strategic \
  --url http://127.0.0.1:8000/v1/chat/completions \
  --api-key dummy \
  --model sut \
  --streaming \
  --prompts ${RESULTS_REMOTE}/prompts/mix.jsonl \
  --max-tokens 512 \
  --ignore-eos \
  --warmup-requests 8 \
  --requests-per-stage 32 \
  --sweep 2,4,8,16,32 \
  --sweep-by rate \
  --max-in-flight 64 \
  --sut ${RESULTS_REMOTE}/sut.json --require-sut \
  --telemetry ${RESULTS_REMOTE}/telemetry.yaml \
  --require-telemetry \
  --ndjson ${RESULTS_REMOTE}/run-open.ndjson \
  --html ${RESULTS_REMOTE}/report-open.html \
  --csv ${RESULTS_REMOTE}/requests-open.csv \
  > ${RESULTS_REMOTE}/stdout-open.json"

# Interrupt: start a long sweep, SIGINT mid-run, expect partial summary.
log "interrupt partial-summary check"
remote "cd ${RESULTS_REMOTE} && ./bin/metrum-ai-bench-cli-strategic \
  --url http://127.0.0.1:8000/v1/chat/completions \
  --api-key dummy \
  --model sut \
  --streaming \
  --prompts ${RESULTS_REMOTE}/prompts/mix.jsonl \
  --max-tokens 512 \
  --ignore-eos \
  --warmup-requests 2 \
  --requests-per-stage 64 \
  --sweep 8,16,32 \
  --sweep-by concurrency \
  --sut ${RESULTS_REMOTE}/sut.json --require-sut \
  --telemetry ${RESULTS_REMOTE}/telemetry.yaml \
  --ndjson ${RESULTS_REMOTE}/run-interrupt.ndjson \
  --html ${RESULTS_REMOTE}/report-interrupt.html \
  --csv ${RESULTS_REMOTE}/requests-interrupt.csv \
  > ${RESULTS_REMOTE}/stdout-interrupt.json &
  echo \$! > ${RESULTS_REMOTE}/interrupt.pid
  sleep 25
  kill -INT \$(cat ${RESULTS_REMOTE}/interrupt.pid) || true
  wait \$(cat ${RESULTS_REMOTE}/interrupt.pid) || true
  python3 - <<'PY'
import json
path='${RESULTS_REMOTE}/run-interrupt.ndjson'
partial=False
kinds=set()
with open(path) as f:
  for line in f:
    row=json.loads(line)
    kinds.add(row.get('kind'))
    if row.get('kind')=='summary':
      partial=bool(row.get('partial'))
assert 'telemetry' in kinds or 'request' in kinds, kinds
assert partial, 'expected summary.partial=true after SIGINT'
print('interrupt ok partial=true kinds=', sorted(kinds))
PY"

# Analyze locally after pull
log "fetching artifacts"
mkdir -p "${ART}/raw"
scp -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -r \
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
  "${ART}/raw/" || true

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

# Validation via analyze.py if present
if [[ -f "${REPO_ROOT}/docs/queries/analyze.py" ]]; then
  python3 "${REPO_ROOT}/docs/queries/analyze.py" "${ART}/raw/run-closed.ndjson" \
    | tee "${ART}/analyze-closed.txt" || true
  python3 "${REPO_ROOT}/docs/queries/analyze.py" "${ART}/raw/run-open.ndjson" \
    | tee "${ART}/analyze-open.txt" || true
fi

cat >"${ART}/VALIDATION.md" <<EOF
<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Shadeform telemetry validation

- timestamp_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)
- model: ${MODEL}
- instance_type: ${TYPE}
- region: ${REGION}
- cloud: ${CLOUD}
- workload: Jarvis-style coding sweep (target ~16k ISL / 512 OSL, concurrency 1..64)
- validation: closed sweep, open rate sweep, SIGINT partial summary
- telemetry: all-smi (Metrum fork /metric), vllm, node, optional dcgm/cadvisor
- artifacts: run-closed/open/interrupt ndjson (compressed), HTML reports, sut.json, telemetry.yaml, stdout JSON

See analyze-closed.txt / analyze-open.txt and README_BUNDLE.md for offline analysis.
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
- `VALIDATION.md`, `cost.txt`, `analyze-*.txt`
- `instance.json`: redacted Shadeform instance metadata

## Offline analysis
1. Decompress: `zstd -d run-closed.ndjson.zst`
2. Read `docs/TELEMETRY.md` and `docs/telemetry/ANALYSIS.md`
3. Run `python3 docs/queries/analyze.py run-closed.ndjson`
4. Or DuckDB: `duckdb -c ".read docs/queries/stage_power.sql"` after setting the input path

Do not invent metric names; use the include list in `telemetry.yaml`.
EOF

log "bundle ready under ${ART}"
kill "${COST_PID}" 2>/dev/null || true
cost_tick || true
