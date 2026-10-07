#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Run the fenced command blocks that documentation marks with an HTML comment.
#
# A block is a fenced ```bash (or ```sh) fence immediately preceded by one of:
#
#   <!-- doc-check -->
#   <!-- doc-check: requires-gpu reason="..." -->
#
# Blocks tagged requires-gpu are reported and skipped. Files without tagged
# blocks are skipped, so this script succeeds on a checkout where the agent
# runbook and AGENTS.md from sibling commits are absent.
#
# Every runnable block starts in a scratch directory with `set -euo pipefail`
# and these variables exported:
#
#   BENCH_BIN_DIR  built debug binaries (target/debug by default)
#   MOCK_URL       base URL of a started metrum-ai-bench-cli-mock-server
#   MOCK_PORT      its port
#   MIRROR         file:// URL of a local release fixture (bootstrap blocks)
#   PREFIX         install prefix for the fixture (bootstrap blocks)
#   VERSION        fixture version string
#
# The fixture provides a cosign shim on PATH so signature verification blocks
# do not need network access.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

BIN_DIR="${BENCH_BIN_DIR:-${ROOT}/target/debug}"
STRICT_MISSING="${DOC_CHECK_STRICT_MISSING:-0}"

# Files this checker scans by default. Missing files are skipped so a branch
# that only carries part of the documentation still passes.
DEFAULT_DOCS=(
  README.md
  AGENTS.md
  llms.txt
  PROPOSALS.md
  docs/INTAKE.md
  docs/AGENT_RUNBOOK.md
  docs/PITFALLS.md
  docs/PROMPT_LIBRARY.md
  docs/STRATEGIC_BENCHMARKING.md
)

if [[ $# -gt 0 ]]; then
  DOCS=("$@")
else
  DOCS=("${DEFAULT_DOCS[@]}")
fi

for bin in metrum-ai-bench-cli-mock-server; do
  if [[ ! -x "${BIN_DIR}/${bin}" ]]; then
    echo "error: missing ${BIN_DIR}/${bin}; run: cargo build --bins" >&2
    exit 1
  fi
done

TMP="$(mktemp -d)"
MOCK_PID=""
cleanup() {
  if [[ -n "${MOCK_PID}" ]] && kill -0 "${MOCK_PID}" 2>/dev/null; then
    kill "${MOCK_PID}" 2>/dev/null || true
    wait "${MOCK_PID}" 2>/dev/null || true
  fi
  rm -rf "${TMP}"
}
trap cleanup EXIT

# Extract tagged blocks into ${TMP}/blocks. Each line of ${TMP}/manifest is:
#   id<TAB>source<TAB>line<TAB>kind<TAB>reason
python3 - "${TMP}" "${DOCS[@]}" <<'PY'
import re
import sys
from pathlib import Path

tmp = Path(sys.argv[1])
blocks = tmp / "blocks"
blocks.mkdir(parents=True, exist_ok=True)
manifest = []

TAG_RE = re.compile(r"^\s*<!--\s*doc-check\s*(?::\s*(.*?))?\s*-->\s*$")
GPU_RE = re.compile(r"requires-gpu")
REASON_RE = re.compile(r'reason\s*=\s*"([^"]*)"')

for raw in sys.argv[2:]:
    path = Path(raw)
    if not path.is_file():
        print(f"check_doc_commands: skip missing {raw}", file=sys.stderr)
        continue
    lines = path.read_text(encoding="utf-8").splitlines()
    i = 0
    while i < len(lines):
        match = TAG_RE.match(lines[i])
        if not match:
            i += 1
            continue
        tag_body = (match.group(1) or "").strip()
        kind = "run"
        reason = ""
        if GPU_RE.search(tag_body):
            kind = "skip"
            reason_match = REASON_RE.search(tag_body)
            reason = reason_match.group(1) if reason_match else "requires-gpu"
        # Find the next fenced code block after the tag.
        j = i + 1
        while j < len(lines) and lines[j].strip() == "":
            j += 1
        if j >= len(lines) or not lines[j].lstrip().startswith("```"):
            print(
                f"check_doc_commands: {raw}:{i + 1}: doc-check tag without a following fenced block",
                file=sys.stderr,
            )
            i += 1
            continue
        info = lines[j].strip()[3:].strip().split()
        lang = info[0] if info else ""
        if lang not in ("bash", "sh", ""):
            print(
                f"check_doc_commands: {raw}:{j + 1}: doc-check fence language {lang!r} is not bash/sh",
                file=sys.stderr,
            )
            i = j + 1
            continue
        body = []
        k = j + 1
        while k < len(lines) and not lines[k].lstrip().startswith("```"):
            body.append(lines[k])
            k += 1
        if k >= len(lines):
            print(
                f"check_doc_commands: {raw}:{j + 1}: unterminated fenced block",
                file=sys.stderr,
            )
            break
        block_id = len(manifest) + 1
        target = blocks / f"{block_id}.sh"
        target.write_text("\n".join(body) + "\n", encoding="utf-8")
        manifest.append((block_id, raw, i + 1, kind, reason))
        i = k + 1

(tmp / "manifest.tsv").write_text(
    "".join(f"{bid}\t{src}\t{line}\t{kind}\t{reason}\n" for bid, src, line, kind, reason in manifest),
    encoding="utf-8",
)
print(f"check_doc_commands: found {len(manifest)} tagged block(s)", file=sys.stderr)
PY

MANIFEST="${TMP}/manifest.tsv"
if [[ ! -s "${MANIFEST}" ]]; then
  echo "check_doc_commands: no tagged blocks; nothing to run"
  exit 0
fi

# --- mock server -----------------------------------------------------------
MOCK_LOG="${TMP}/mock.log"
"${BIN_DIR}/metrum-ai-bench-cli-mock-server" --listen 127.0.0.1:0 --telemetry-fixture \
  >"${MOCK_LOG}" 2>&1 &
MOCK_PID=$!
MOCK_PORT=""
for _ in $(seq 1 100); do
  if grep -q 'listening on' "${MOCK_LOG}" 2>/dev/null; then
    MOCK_PORT="$(sed -n 's/.*listening on [^:]*:\([0-9]*\).*/\1/p' "${MOCK_LOG}" | head -n 1)"
    break
  fi
  if ! kill -0 "${MOCK_PID}" 2>/dev/null; then
    echo "error: mock server exited during startup" >&2
    cat "${MOCK_LOG}" >&2 || true
    exit 1
  fi
  sleep 0.05
done
if [[ -z "${MOCK_PORT}" ]]; then
  echo "error: mock server did not report a listening port" >&2
  cat "${MOCK_LOG}" >&2 || true
  exit 1
fi
MOCK_URL="http://127.0.0.1:${MOCK_PORT}"
for _ in $(seq 1 100); do
  if curl --silent --fail "${MOCK_URL}/health" >/dev/null 2>&1; then
    break
  fi
  sleep 0.05
done

# --- release fixture (for bootstrap blocks) --------------------------------
FIXTURE="${TMP}/fixture"
SHIMBIN="${TMP}/shimbin"
mkdir -p "${FIXTURE}/bin" "${SHIMBIN}" "${TMP}/prefix"
for bin in metrum-ai-bench-cli metrum-ai-bench-cli-llm metrum-ai-bench-cli-vlm \
  metrum-ai-bench-cli-asr metrum-ai-bench-cli-imagegen metrum-ai-bench-cli-prompts \
  metrum-ai-bench-cli-strategic metrum-ai-bench-cli-mock-server; do
  if [[ -x "${BIN_DIR}/${bin}" ]]; then
    ln -sf "${BIN_DIR}/${bin}" "${FIXTURE}/bin/${bin}"
  fi
done
ln -sfn "${ROOT}/test-data" "${FIXTURE}/repo-test-data"
ASSET="metrum-ai-bench-cli-fixture.tar.gz"
# Store symlinks (no -h) so the fixture stays cheap to build.
tar -czf "${FIXTURE}/${ASSET}" -C "${FIXTURE}" bin repo-test-data
( cd "${FIXTURE}" && sha256sum "${ASSET}" >"${ASSET}.sha256" )
cat >"${SHIMBIN}/cosign" <<'SHIM'
#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
# Offline shim: documentation command checks do not verify real signatures.
exit 0
SHIM
chmod +x "${SHIMBIN}/cosign"
MIRROR="file://${FIXTURE}"
PREFIX="${TMP}/prefix"
VERSION="fixture"
REPO_ROOT="${ROOT}"

# Fixture campaign for length_check / validity blocks (mock chat usage is 8/4).
CAMPAIGN="${TMP}/campaign"
mkdir -p "${CAMPAIGN}"
cat >"${CAMPAIGN}/run.ndjson" <<'NDJSON'
{"schema_version":"telemetry.v1","kind":"request","stage":"c1","warmup":false,"success":true,"input_tokens":8,"output_tokens":4}
{"schema_version":"telemetry.v1","kind":"request","stage":"c1","warmup":false,"success":true,"input_tokens":8,"output_tokens":4}
NDJSON
ISL_TARGET=8
ISL_TOLERANCE=0
OSL_TARGET=4
OSL_TOLERANCE=0

export BENCH_BIN_DIR MOCK_URL MOCK_PORT MIRROR PREFIX VERSION REPO_ROOT CAMPAIGN
export ISL_TARGET ISL_TOLERANCE OSL_TARGET OSL_TOLERANCE
export PATH="${SHIMBIN}:${PATH}"

# --- run blocks ------------------------------------------------------------
status=0
skipped=0
ran=0
while IFS=$'\t' read -r bid src line kind reason; do
  if [[ "${kind}" == "skip" ]]; then
    echo "check_doc_commands: skip ${src}:${line} (${reason})"
    skipped=$((skipped + 1))
    continue
  fi
  echo "check_doc_commands: run ${src}:${line} (block ${bid})"
  if ! ( cd "${REPO_ROOT}" && bash "${TMP}/blocks/${bid}.sh" ); then
    echo "check_doc_commands: FAILED ${src}:${line}" >&2
    status=1
  fi
  ran=$((ran + 1))
done <"${MANIFEST}"

echo "check_doc_commands: ran ${ran}, skipped ${skipped}"
exit "${status}"
