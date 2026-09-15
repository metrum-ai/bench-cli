#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# End-of-work GPU campaign: launch retained parallel Shadeform lanes,
# run sweeps, validate, and write a consolidated smoke report.
# Dry-run unless --execute. Does not delete instances; use "teardown"
# after validate + report (docs/SMOKE_RESULTS.md) are done.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RESULTS_DIR="${RESULTS_DIR:-${REPO_ROOT}/live-results}"
SHADE="${SCRIPT_DIR}/shadeform.sh"
LANES="${CAMPAIGN_LANES:-llm vlm}"
LLM_MODEL="${LLM_MODEL:-Qwen/Qwen2.5-7B-Instruct}"
VLM_MODEL="${VLM_MODEL:-Qwen/Qwen2.5-VL-7B-Instruct}"
HOST_PORT="${HOST_PORT:-80}"

die() { echo "error: $*" >&2; exit 1; }

campaign_id="${CAMPAIGN_ID:-$(date -u +%Y%m%d-%H%M%S)}"
root="${RESULTS_DIR}/campaign-${campaign_id}"
execute=0

usage() {
  cat <<'EOF'
Usage: campaign.sh [--execute] <command>

Commands:
  plan       Print the sweep matrix and instance layout (no API)
  launch     Create one retained instance per lane (parallel POSTs)
  sweep      Run the retained matrix against instances.json
  validate   Schema / completeness checks on live-results
  report     Consolidate modality results + SUT provenance into docs/SMOKE_RESULTS.md
  teardown   Delete every id in instances.json
  demo       Print public demo commands (no spend)

Default is dry-run. --execute is required to create, sweep, or delete.
Keep instances until validate + report (docs/SMOKE_RESULTS.md) are done.
Artifacts ship via GitHub Releases — no private backup step.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --execute) execute=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) break ;;
  esac
done

cmd="${1:-plan}"
shift || true

resolve_bin() {
  local want="$1"
  local candidate
  for candidate in \
    "${REPO_ROOT}/target/release/${want}" \
    "${REPO_ROOT}/target/debug/${want}"; do
    if [[ -x "${candidate}" ]]; then
      echo "${candidate}"
      return 0
    fi
  done
  if command -v "${want}" >/dev/null 2>&1; then
    command -v "${want}"
    return 0
  fi
  die "binary not found: ${want}"
}

wait_http() {
  local url="$1"
  local max="${2:-90}"
  local i
  for ((i = 1; i <= max; i++)); do
    if curl -fsS -o /dev/null --max-time 5 "${url}"; then
      echo "# ready ${url}" >&2
      return 0
    fi
    echo "# wait ${i}/${max} ${url}" >&2
    sleep 10
  done
  die "timed out waiting for ${url}"
}

sha256_tree() {
  local dir="$1"
  (
    cd "${dir}"
    find . -type f ! -name sha256sums -print0 | sort -z | xargs -0 sha256sum
  ) >"${dir}/sha256sums"
}

cmd_plan() {
  mkdir -p "${root}"
  cat <<EOF
campaign_id=${campaign_id}
root=${root}
lanes=${LANES}
execute=${execute}

LLM cells:
  concurrency 1 2 4 8   closed-loop  warmup=8 n=64 max_tokens=128 seed=7
  request-rate 4 8 16   arrival=constant max-concurrency=8

VLM cells:
  concurrency 1 2 4     streaming     warmup=8 n=32 max_tokens=64 seed=7

ASR / imagegen:
  launched only if CAMPAIGN_LANES includes asr or imagegen and an image is
  practical; otherwise dummy-certified and labeled in manifest.json.

Instances stay up until: campaign.sh teardown --execute
EOF
}

cmd_launch() {
  mkdir -p "${root}"
  # Ensure the API key is in the environment for parallel create children.
  if [[ -z "${SHADEFORM_API_KEY:-}" && -f "${REPO_ROOT}/env.json" ]]; then
    SHADEFORM_API_KEY="$(jq -r '.SHADEFORM_API_KEY // empty' "${REPO_ROOT}/env.json")"
    export SHADEFORM_API_KEY
  fi
  if [[ "${execute}" -eq 0 ]]; then
    echo "# dry-run launch lanes: ${LANES}"
    for lane in ${LANES}; do
      echo "# would: ${SHADE} create --engine vllm --modality ${lane} --name metrum-${campaign_id}-${lane} --execute"
    done
    echo "# write ${root}/instances.json after real create"
    return 0
  fi
  local pids=() names=()
  mkdir -p "${root}/launch-logs"
  for lane in ${LANES}; do
    case "${lane}" in
      llm|vlm) ;;
      *)
        echo "# skip create for ${lane}: shadeform.sh create supports llm|vlm only" >&2
        continue
        ;;
    esac
    local name="metrum-${campaign_id}-${lane}"
    names+=("${lane}")
    (
      # Response-only file: strip the dry-run style payload chatter by taking
      # the last JSON object containing an id.
      "${SHADE}" create --engine vllm --modality "${lane}" --name "${name}" --execute \
        >"${root}/launch-logs/${lane}.raw" 2>"${root}/launch-logs/${lane}.err"
      python3 - "${root}/launch-logs/${lane}.raw" "${root}/launch-logs/${lane}.json" <<'PY'
import json, pathlib, re, sys
raw = pathlib.Path(sys.argv[1]).read_text()
ids = re.findall(r'"id"\s*:\s*"([0-9a-f-]{36})"', raw)
pathlib.Path(sys.argv[2]).write_text(json.dumps({"id": ids[-1]}) + "\n" if ids else "{}")
if not ids:
    raise SystemExit(f"no instance id in {sys.argv[1]}")
PY
    ) &
    pids+=("$!")
  done
  local status=0
  local pid
  for pid in "${pids[@]+"${pids[@]}"}"; do
    wait "${pid}" || status=1
  done
  [[ "${status}" -eq 0 ]] || die "one or more parallel creates failed; inspect ${root}/launch-logs"
  local entries="[]"
  local lane id
  for lane in "${names[@]+"${names[@]}"}"; do
    id="$(jq -r '.id // empty' "${root}/launch-logs/${lane}.json")"
    [[ -n "${id}" ]] || die "no instance id in launch-logs/${lane}.json"
    entries="$(jq --arg lane "${lane}" --arg id "${id}" \
      '. + [{lane:$lane,id:$id,status:"created"}]' <<<"${entries}")"
  done
  echo "${entries}" | jq . >"${root}/instances.json"
  echo "# launched; waiting for IPs"
  local row ip
  local waited="[]"
  while read -r row; do
    lane="$(jq -r .lane <<<"${row}")"
    id="$(jq -r .id <<<"${row}")"
    ip="$("${SHADE}" wait "${id}")"
    waited="$(jq --arg lane "${lane}" --arg id "${id}" --arg ip "${ip}" \
      '. + [{lane:$lane,id:$id,ip:$ip,status:"ready",port:"'"${HOST_PORT}"'"}]' <<<"${waited}")"
  done < <(jq -c '.[]' "${root}/instances.json")
  echo "${waited}" | jq . >"${root}/instances.json"
  jq -n --arg id "${campaign_id}" --arg root "${root}" \
    '{campaign_id:$id,root:$root,partial:false,note:"instances retained until teardown"}' \
    >"${root}/manifest.json"
  echo "${root}/instances.json"
}

cmd_teardown() {
  local file="${root}/instances.json"
  [[ -f "${file}" ]] || die "missing ${file} (set CAMPAIGN_ID to the campaign directory suffix)"
  if [[ "${execute}" -eq 0 ]]; then
    echo "# dry-run teardown:"
    jq -r '.[] | "would delete \(.id) lane=\(.lane)"' "${file}"
    return 0
  fi
  local id
  while read -r id; do
    [[ -n "${id}" ]] || continue
    "${SHADE}" delete "${id}" || true
  done < <(jq -r '.[].id' "${file}")
}

cmd_demo() {
  cat <<'EOF'
# After a campaign directory exists (gitignored live-results/):

metrum-ai-bench-llm \
  --url http://HOST/v1/chat/completions --api-key dummy \
  --scenario demo-llm --num-requests 64 --concurrency 4 \
  --warmup-requests 8 --seed 7 --streaming --mode chat \
  --prompts prompts.jsonl --model Qwen/Qwen2.5-7B-Instruct \
  --max-tokens 128 --data-log demo-llm.jsonl

metrum-ai-bench-vlm \
  --url http://HOST/v1/chat/completions --api-key dummy \
  --scenario demo-vlm --num-requests 32 --concurrency 2 \
  --warmup-requests 8 --seed 7 --streaming \
  --prompts vlm.jsonl --model Qwen/Qwen2.5-VL-7B-Instruct \
  --max-tokens 64 --data-log demo-vlm.jsonl

# Open-loop LLM:
metrum-ai-bench-llm ... --request-rate 8 --arrival constant --max-concurrency 8
EOF
}

cmd_validate() {
  local dir="${1:-${root}}"
  [[ -d "${dir}" ]] || die "campaign dir missing: ${dir} (run after sweeps)"
  find "${dir}" -name 'results.jsonl' | grep -q . || die "no results.jsonl under ${dir}"
  python3 - "${dir}" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
errors = []
cells = 0
request_lines = 0
for path in root.rglob("results.jsonl"):
    cells += 1
    n = 0
    saw_request = False
    for line in path.read_text().splitlines():
        if not line.strip():
            continue
        n += 1
        rec = json.loads(line)
        schema = rec.get("schema_version")
        # New campaigns reject unversioned / legacy lines (fail, do not skip).
        # request.v2 / summary.v2 from 0.1.82 remain accepted for regression audit.
        if schema is None:
            errors.append(f"{path}: line {n} missing schema_version (legacy/unversioned JSONL rejected)")
            continue
        schema_s = str(schema)
        if "request" in schema_s:
            saw_request = True
            request_lines += 1
            if not any(v in schema_s for v in ("request.v2", "request.v3", "imagegen.request")):
                errors.append(f"{path}: line {n} unsupported request schema_version={schema_s}")
        elif "summary" in schema_s:
            if not any(v in schema_s for v in ("summary.v2", "summary.v3", "imagegen.summary")):
                errors.append(f"{path}: line {n} unsupported summary schema_version={schema_s}")
        else:
            errors.append(f"{path}: line {n} unrecognized schema_version={schema_s}")
    if n == 0:
        errors.append(f"{path}: empty")
    if not saw_request and "imagegen" not in str(path):
        # imagegen writes modality request.v1 rows; others must have request.v2
        if not any("schema_version" in json.loads(l) for l in path.read_text().splitlines() if l.strip()):
            errors.append(f"{path}: no schema_versioned records")
if cells == 0:
    errors.append("no results.jsonl files")
if request_lines == 0:
    # imagegen-only campaigns still count as success if schema present
    pass
if errors:
    print("\n".join(errors), file=sys.stderr)
    sys.exit(1)
print(f"ok: {cells} result files under {root}; {request_lines} request lines (v2/v3)")
PY
}

cmd_report() {
  local dir="${1:-${root}}"
  local out="${2:-${REPO_ROOT}/docs/SMOKE_RESULTS.md}"
  [[ -d "${dir}" ]] || die "campaign dir missing: ${dir}"
  python3 - "${dir}" "${out}" "${campaign_id}" <<'PY'
import json, pathlib, statistics, sys
from datetime import datetime, timezone

root = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])
campaign_id = sys.argv[3]
manifest = {}
if (root / "manifest.json").exists():
    manifest = json.loads((root / "manifest.json").read_text())
# Drop any legacy restic fields from private manifest.
manifest.pop("restic_snapshot_id", None)
if manifest:
    (root / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
for stale in ("restic-snapshot.json", "backup.log"):
    p = root / stale
    if p.exists():
        p.unlink()

def pct(xs, p):
    if not xs:
        return None
    xs = sorted(xs)
    if len(xs) == 1:
        return xs[0]
    k = (len(xs) - 1) * p / 100.0
    f = int(k)
    c = min(f + 1, len(xs) - 1)
    if f == c:
        return xs[f]
    return xs[f] * (c - k) + xs[c] * (k - f)

def fmt(v, digits=3):
    if v is None:
        return "—"
    return f"{v:.{digits}f}"

rows = []
request_lines = 0
cells = 0
for path in sorted(root.rglob("results.jsonl")):
    cells += 1
    parts = path.relative_to(root).parts
    modality = parts[0] if parts else "unknown"
    cell = parts[1] if len(parts) > 1 else path.parent.name
    lat, ttft, rtfx, wer = [], [], [], []
    for line in path.read_text().splitlines():
        if not line.strip():
            continue
        rec = json.loads(line)
        if rec.get("phase") == "warmup":
            continue
        schema = str(rec.get("schema_version", ""))
        if "request" not in schema and "latency_s" not in rec:
            continue
        request_lines += 1
        if isinstance(rec.get("latency_s"), (int, float)):
            lat.append(rec["latency_s"])
        if isinstance(rec.get("ttft_s"), (int, float)):
            ttft.append(rec["ttft_s"])
        mm = rec.get("modality_metrics") or {}
        if isinstance(mm.get("rtfx_client"), (int, float)):
            rtfx.append(mm["rtfx_client"])
        if isinstance(mm.get("wer"), (int, float)):
            wer.append(mm["wer"])
        elif isinstance(rec.get("wer"), (int, float)):
            wer.append(rec["wer"])
    rows.append({
        "modality": modality,
        "cell": cell,
        "n": len(lat) or len(rtfx) or len(wer),
        "latency_p50": pct(lat, 50),
        "latency_p95": pct(lat, 95),
        "ttft_p50": pct(ttft, 50),
        "ttft_p95": pct(ttft, 95),
        "rtfx_p50": pct(rtfx, 50),
        "wer_p50": pct(wer, 50),
    })

(root / "aggregate.json").write_text(json.dumps(rows, indent=2) + "\n")

imagegen_rows = []
for summ in sorted(root.glob("imagegen/*/summary.json")):
    s = json.loads(summ.read_text())
    ep = (s.get("endpoints") or [{}])[0]
    imagegen_rows.append({
        "cell": summ.parent.name,
        "images_per_second": ep.get("images_per_second"),
        "latency_ms_p50": ep.get("latency_ms_p50"),
        "successful_requests": ep.get("successful_requests"),
    })

sut_path = root / "sut.json"
sut = json.loads(sut_path.read_text()) if sut_path.exists() else {}
bench_ver = "0.1.82"
now = datetime.now(timezone.utc).strftime("%Y-%m-%d")

def table(headers, body_lines):
    sep = "| " + " | ".join("---" if h.endswith(":") or h in ("n",) or "p5" in h or "p50" in h or "p95" in h or "/s" in h or "(ms)" in h or "(s)" in h else "---" for h in headers) + " |"
    # simpler separator
    sep = "| " + " | ".join(["---"] * len(headers)) + " |"
    head = "| " + " | ".join(headers) + " |"
    return "\n".join([head, sep] + body_lines)

llm_closed = [r for r in rows if r["modality"] == "llm" and r["cell"].startswith("c")]
llm_rate = [r for r in rows if r["modality"] == "llm" and r["cell"].startswith("rate")]
vlm = [r for r in rows if r["modality"] == "vlm"]
asr = [r for r in rows if r["modality"] == "asr"]

def lat_rows(rs):
    out_lines = []
    for r in rs:
        out_lines.append(
            f"| {r['cell']} | {r['n']} | {fmt(r['latency_p50'])} | {fmt(r['latency_p95'])} | "
            f"{fmt(r['ttft_p50'])} | {fmt(r['ttft_p95'])} |"
        )
    return out_lines

asr_lines = []
for r in asr:
    asr_lines.append(
        f"| {r['cell']} | {r['n']} | {fmt(r['latency_p50'])} | {fmt(r['rtfx_p50'], 2)} | {fmt(r['wer_p50'], 1)} |"
    )
img_lines = []
for r in imagegen_rows:
    img_lines.append(
        f"| {r['cell']} | {r.get('successful_requests') or '—'} | "
        f"{fmt(r.get('images_per_second'))} | {fmt(r.get('latency_ms_p50'))} |"
    )

gpu = sut.get("gpu") or {}
server = sut.get("model_server") or {}
lines = [
    "<!-- Copyright (c) 2026 Metrum AI, Inc. -->",
    "<!-- SPDX-License-Identifier: Apache-2.0 -->",
    "",
    f"# Smoke results — campaign `{campaign_id}`",
    "",
    "Consolidated multi-modality smoke after local gates. Raw JSONL stays under",
    "gitignored `live-results/`; this document is the public, releasable summary",
    "(SUT provenance + aggregates). Ship via **GitHub Releases**.",
    "",
    "| Field | Value |",
    "|-------|-------|",
    f"| Campaign ID | `{campaign_id}` |",
    f"| Bench package | `metrum-ai-bench-*` **{bench_ver}** |",
    f"| Date (UTC) | {now} |",
    f"| Validation | {cells} result files, {request_lines} measured request lines |",
    f"| Modalities | {', '.join(manifest.get('modalities_complete') or sorted({r['modality'] for r in rows}))} |",
    "",
    "## Systems under test",
    "",
    "### GPU lanes (LLM / VLM)",
    "",
    "| Item | Value |",
    "|------|-------|",
    f"| Cloud / region | {sut.get('cloud', 'Shadeform')} / {sut.get('region', '—')} |",
    f"| Instance type | {sut.get('shade_instance_type', '—')} / {sut.get('cloud_instance_type', '—')} |",
    f"| GPU | {gpu.get('name', '—')} ×{gpu.get('count', 1)} |",
    f"| VRAM | {gpu.get('vram_gib', '—')} GiB |",
    f"| Host OS | {sut.get('os', '—')} |",
    f"| NVIDIA driver | **{gpu.get('driver', '—')}** |",
    f"| Host CUDA (driver) | **{gpu.get('cuda', '—')}** |",
    f"| Model server | **{server.get('name', 'vLLM')} {server.get('version', '—')}** |",
    f"| Container image | `{server.get('image', '—')}` |",
    f"| Image digest | `{server.get('digest', '—')}` |",
    f"| Torch / CUDA (container) | {server.get('torch', '—')} / {server.get('torch_cuda', '—')} |",
    f"| LLM model | `{sut.get('llm_model', 'Qwen/Qwen2.5-7B-Instruct')}` |",
    f"| VLM model | `{sut.get('vlm_model', 'Qwen/Qwen2.5-VL-7B-Instruct')}` |",
    "",
    "### ASR / imagegen",
    "",
    "Labeled **dummy-certified** when no practical GPU image was available;",
    "validates CLI wiring and schemas only.",
    "",
    "## Results",
    "",
    "### LLM — closed-loop concurrency",
    "",
    table(
        ["Cell", "n", "latency p50", "latency p95", "TTFT p50", "TTFT p95"],
        lat_rows(llm_closed),
    ),
    "",
    "### LLM — open-loop request rate",
    "",
    table(
        ["Cell", "n", "latency p50", "latency p95", "TTFT p50", "TTFT p95"],
        lat_rows(llm_rate),
    ),
    "",
    "### VLM — concurrency",
    "",
    table(
        ["Cell", "n", "latency p50", "latency p95", "TTFT p50", "TTFT p95"],
        lat_rows(vlm),
    ),
    "",
    "### ASR — dummy-certified",
    "",
    table(
        ["Cell", "n", "latency p50 (s)", "RTFx client p50", "WER p50"],
        asr_lines or ["| — | — | — | — | — |"],
    ),
    "",
    "### Imagegen — dummy-certified",
    "",
    table(
        ["Cell", "Successful images", "Images/s", "Latency p50 (ms)"],
        img_lines or ["| — | — | — | — |"],
    ),
    "",
    "## Reproducing",
    "",
    "```bash",
    "./scripts/live/campaign.sh demo",
    "./scripts/live/campaign.sh validate",
    "./scripts/live/campaign.sh report",
    "```",
    "",
]
out.parent.mkdir(parents=True, exist_ok=True)
out.write_text("\n".join(lines) + "\n")
print(f"# wrote {out}")
print(f"# refreshed {root / 'aggregate.json'} ({len(rows)} cells)")
PY
}

write_llm_prompts() {
  local path="$1"
  local n="$2"
  local i
  : >"${path}"
  for ((i = 0; i < n; i++)); do
    printf '{"prompt":"Campaign prompt %s. Reply in one short sentence about throughput testing."}\n' "${i}" >>"${path}"
  done
}

write_vlm_prompts() {
  local path="$1"
  local img="$2"
  local n="$3"
  local i
  : >"${path}"
  for ((i = 0; i < n; i++)); do
    jq -nc --arg p "Describe this image in one short sentence. cell=${i}" --arg u "${img}" \
      '{prompt:$p, image_url:$u}' >>"${path}"
  done
}

run_llm_cell() {
  local url="$1" cell="$2" conc="$3" nreq="$4"
  shift 4
  local out="${root}/llm/${cell}"
  mkdir -p "${out}"
  local prompts="${out}/prompts.jsonl"
  write_llm_prompts "${prompts}" "$((nreq + 16))"
  local bin
  bin="$(resolve_bin metrum-ai-bench-llm)"
  {
    echo "${bin}"
    printf ' %q' --url "${url}" --api-key none --scenario "campaign-llm-${cell}" \
      --num-requests "${nreq}" --concurrency "${conc}" --warmup-requests 8 --seed 7 \
      --mode chat --streaming --prompts "${prompts}" --model "${LLM_MODEL}" \
      --max-tokens 128 --data-log "${out}/results.jsonl" \
      --debug-log "${out}/debug.log" --error-log "${out}/error.log" --log-level warn \
      "$@"
    echo
  } >"${out}/command.txt"
  # shellcheck disable=SC2086
  "${bin}" --url "${url}" --api-key none --scenario "campaign-llm-${cell}" \
    --num-requests "${nreq}" --concurrency "${conc}" --warmup-requests 8 --seed 7 \
    --mode chat --streaming --prompts "${prompts}" --model "${LLM_MODEL}" \
    --max-tokens 128 --data-log "${out}/results.jsonl" \
    --debug-log "${out}/debug.log" --error-log "${out}/error.log" --log-level warn \
    "$@" | tee "${out}/stdout.txt"
  sha256_tree "${out}"
}

run_vlm_cell() {
  local url="$1" cell="$2" conc="$3" nreq="$4"
  local out="${root}/vlm/${cell}"
  mkdir -p "${out}"
  local img="${root}/fixtures/pixel.png"
  mkdir -p "${root}/fixtures"
  if [[ ! -f "${img}" ]]; then
    python3 - <<'PY' "${img}"
import struct, zlib, pathlib, sys
def chunk(tag, data):
    return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
sig = b"\x89PNG\r\n\x1a\n"
ihdr = chunk(b"IHDR", struct.pack(">IIBBBBB", 2, 2, 8, 2, 0, 0, 0))
# filter byte + RGB for 2 pixels per row, two rows
raw = b"\x00\xff\x00\x00\x00\xff\x00" + b"\x00\x00\xff\x00\xff\x00\x00"
idat = chunk(b"IDAT", zlib.compress(raw))
iend = chunk(b"IEND", b"")
pathlib.Path(sys.argv[1]).write_bytes(sig + ihdr + idat + iend)
PY
  fi
  local prompts="${out}/prompts.jsonl"
  write_vlm_prompts "${prompts}" "${img}" "$((nreq + 8))"
  local bin
  bin="$(resolve_bin metrum-ai-bench-vlm)"
  {
    echo "${bin}"
    printf ' %q' --url "${url}" --api-key none --scenario "campaign-vlm-${cell}" \
      --num-requests "${nreq}" --concurrency "${conc}" --warmup-requests 8 --seed 7 \
      --streaming --prompts "${prompts}" --model "${VLM_MODEL}" --max-tokens 64 \
      --data-log "${out}/results.jsonl" --debug-log "${out}/debug.log" \
      --error-log "${out}/error.log" --log-level warn
    echo
  } >"${out}/command.txt"
  "${bin}" --url "${url}" --api-key none --scenario "campaign-vlm-${cell}" \
    --num-requests "${nreq}" --concurrency "${conc}" --warmup-requests 8 --seed 7 \
    --streaming --prompts "${prompts}" --model "${VLM_MODEL}" --max-tokens 64 \
    --data-log "${out}/results.jsonl" --debug-log "${out}/debug.log" \
    --error-log "${out}/error.log" --log-level warn | tee "${out}/stdout.txt"
  sha256_tree "${out}"
}

cmd_sweep() {
  echo "# campaign ${campaign_id} lanes=${LANES} execute=${execute}"
  echo "# LLM: c=1,2,4,8 closed-loop; rate=4,8,16 constant; warmup=8 n=64 seed=7"
  echo "# VLM: c=1,2,4 streaming; warmup=8 n=32 seed=7"
  if [[ "${execute}" -eq 0 ]]; then
    echo "# dry-run: not contacting GPUs. Re-run with --execute after local gates."
    return 0
  fi
  [[ -f "${root}/instances.json" ]] || die "missing ${root}/instances.json (launch --execute first)"

  local llm_ip vlm_ip llm_port vlm_port
  llm_ip="$(jq -r '.[] | select(.lane=="llm") | .ip' "${root}/instances.json")"
  vlm_ip="$(jq -r '.[] | select(.lane=="vlm") | .ip' "${root}/instances.json")"
  llm_port="$(jq -r '.[] | select(.lane=="llm") | .port // "8000"' "${root}/instances.json")"
  vlm_port="$(jq -r '.[] | select(.lane=="vlm") | .port // "8000"' "${root}/instances.json")"

  if [[ -n "${llm_ip}" && "${llm_ip}" != "null" ]]; then
    wait_http "http://${llm_ip}:${llm_port}/v1/models" 120
    local llm_url="http://${llm_ip}:${llm_port}/v1/chat/completions"
    local c
    for c in 1 2 4 8; do
      echo "# LLM closed-loop concurrency=${c}" >&2
      run_llm_cell "${llm_url}" "c${c}-n64" "${c}" 64
    done
    local rate
    for rate in 4 8 16; do
      echo "# LLM open-loop rate=${rate}" >&2
      run_llm_cell "${llm_url}" "rate${rate}-n64" 8 64 \
        --request-rate "${rate}" --arrival constant --max-concurrency 8
    done
  fi

  if [[ -n "${vlm_ip}" && "${vlm_ip}" != "null" ]]; then
    wait_http "http://${vlm_ip}:${vlm_port}/v1/models" 120
    local vlm_url="http://${vlm_ip}:${vlm_port}/v1/chat/completions"
    local c
    for c in 1 2 4; do
      echo "# VLM concurrency=${c}" >&2
      run_vlm_cell "${vlm_url}" "c${c}-n32" "${c}" 32
    done
  fi

  jq --argjson now "$(date -u +%s)" \
    '. + {swept_at_unix:$now, lanes_completed:["llm","vlm"]}' \
    "${root}/manifest.json" >"${root}/manifest.json.tmp"
  mv "${root}/manifest.json.tmp" "${root}/manifest.json"
  echo "# sweep complete under ${root}"
}

case "${cmd}" in
  plan) cmd_plan ;;
  launch) cmd_launch ;;
  teardown) cmd_teardown ;;
  demo) cmd_demo ;;
  validate) cmd_validate "$@" ;;
  report) cmd_report "$@" ;;
  sweep) cmd_sweep ;;
  *) die "unknown command: ${cmd}" ;;
esac
