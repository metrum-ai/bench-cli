#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Metrum AI Bench CLI live gate: fail a smoke cell whose data log could not
# back a headline claim for its modality.
#
# Usage: assert_headline.sh <llm|vlm|asr|imagegen> <data_log.jsonl> [--artifact-dir DIR]
#
# Exits nonzero when any of these holds:
#   - the data log has no summary.v3 line, or the run did not set --require-sut
#   - measured successes are 0, or successes / attempted is below
#     MIN_SUCCESS_RATIO (default 1.0, meaning every measured request succeeded)
#   - asr: a successful measured record lacks wer or cer (summary.v3 carries
#     no WER aggregate, so this checks every request record and prints means)
#   - vlm: a successful measured record has image_count 0 or missing
#   - imagegen: no successful record returned images, or (with --artifact-dir)
#     no artifact file decodes as PNG (zlib IDAT inflates) or JPEG (SOI..EOI)
# Requires python3 (standard library only).
set -euo pipefail

usage() { sed -n '5,18p' "$0" >&2; exit 2; }
[[ $# -ge 2 ]] || usage
modality="$1"; log="$2"; shift 2
artifact_dir=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --artifact-dir) artifact_dir="$2"; shift 2 ;;
    *) usage ;;
  esac
done
case "${modality}" in llm|vlm|asr|imagegen) ;; *) usage ;; esac
[[ -f "${log}" ]] || { echo "assert_headline: FAIL ${modality}: no data log at ${log}" >&2; exit 1; }

exec python3 - "${modality}" "${log}" "${artifact_dir}" "${MIN_SUCCESS_RATIO:-1.0}" <<'PY'
import json
import pathlib
import sys
import zlib

modality, log, artifact_dir, min_ratio = sys.argv[1], sys.argv[2], sys.argv[3], float(sys.argv[4])
failures = []
rows = []
for n, line in enumerate(pathlib.Path(log).read_text().splitlines(), 1):
    if line.strip():
        try:
            rows.append(json.loads(line))
        except json.JSONDecodeError as e:
            failures.append(f"line {n} is not JSON: {e}")

def schema(r):
    return r.get("schema_version") or ""

summary = next((r for r in reversed(rows) if "summary.v" in schema(r)), None)
measured = [r for r in rows if "request.v" in schema(r) and r.get("phase", "measure") == "measure"]
ok = [r for r in measured if not r.get("error")]

if summary is None:
    failures.append("no summary.v3 line (run crashed or was killed)")
else:
    common = (summary.get("config") or {}).get("common") or {}
    if common.get("require_sut") is not True:
        failures.append("run did not set --require-sut")
    if not summary.get("sut"):
        failures.append("summary has no sut block")
    attempted = summary.get("attempted") or 0
    successes = summary.get("successes") or 0
    if successes == 0:
        failures.append(f"0 successes out of {attempted} attempted (errors_by_type={summary.get('errors_by_type')})")
    elif attempted and successes / attempted < min_ratio:
        failures.append(f"success ratio {successes}/{attempted} below MIN_SUCCESS_RATIO={min_ratio}")

def metric(r, key):
    return (r.get("modality_metrics") or {}).get(key)

extra = ""
if modality == "asr" and ok:
    missing = [r.get("seq") for r in ok if metric(r, "wer") is None or metric(r, "cer") is None]
    if missing:
        failures.append(f"{len(missing)} successful ASR records lack wer/cer (pass --ground-truth); seq={missing[:5]}")
    else:
        wer = sum(metric(r, "wer") for r in ok) / len(ok)
        cer = sum(metric(r, "cer") for r in ok) / len(ok)
        extra = f" mean_wer={wer:.4f} mean_cer={cer:.4f} n={len(ok)}"
elif modality == "vlm" and ok:
    zero = [r.get("seq") for r in ok if not metric(r, "image_count")]
    if zero:
        failures.append(f"{len(zero)} successful VLM records sent no image (image_count 0); seq={zero[:5]}")
elif modality == "imagegen":
    returned = sum(metric(r, "images_returned") or 0 for r in ok)
    if returned == 0:
        failures.append("no successful imagegen record returned images")
    if artifact_dir:
        def decodes(p):
            b = p.read_bytes()
            if b.startswith(b"\x89PNG\r\n\x1a\n"):
                pos, idat, w = 8, b"", 0
                while pos + 8 <= len(b):
                    size = int.from_bytes(b[pos:pos + 4], "big")
                    kind = b[pos + 4:pos + 8]
                    data = b[pos + 8:pos + 8 + size]
                    if kind == b"IHDR":
                        w = int.from_bytes(data[0:4], "big") * int.from_bytes(data[4:8], "big")
                    elif kind == b"IDAT":
                        idat += data
                    pos += 12 + size
                try:
                    return w > 0 and len(zlib.decompress(idat)) > 0
                except zlib.error:
                    return False
            return b.startswith(b"\xff\xd8") and b.rstrip(b"\x00").endswith(b"\xff\xd9")
        files = [p for p in pathlib.Path(artifact_dir).rglob("*") if p.is_file() and p.suffix.lower() in (".png", ".jpg", ".jpeg")]
        good = [p for p in files if decodes(p)]
        if not good:
            failures.append(f"no decodable PNG/JPEG artifacts in {artifact_dir} ({len(files)} image files)")
        extra = f" decoded_artifacts={len(good)}/{len(files)}"

status = "FAIL" if failures else "PASS"
n_ok, n_meas = len(ok), len(measured)
print(f"assert_headline: {status} {modality} {log} measured_ok={n_ok}/{n_meas}{extra}")
for f in failures:
    print(f"  - {f}")
sys.exit(1 if failures else 0)
PY
