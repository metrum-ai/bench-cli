#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Metrum AI Bench CLI: offline regression test for docs/queries/analyze.py.
# all-smi exports power_limit_{current,max}_watts beside the power draw gauge
# and an energy counter in millijoules. Power must ignore the limits, and the
# counter must come out in joules whether or not the ingest scaled it.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

python3 - "${work}" <<'PY'
import json, pathlib, sys
w = pathlib.Path(sys.argv[1])
def rows(scaled):
    out = [{"kind": "run", "run_id": "r"},
           {"kind": "stage", "run_id": "r", "phase": "measure", "stage": 1.0, "load": 1.0,
            "t_start_ns": 0, "t_end_ns": 10_000_000_000}]
    for i in range(11):
        t = i * 1_000_000_000
        out.append({"kind": "telemetry", "run_id": "r", "t_ns": t, "src": "all-smi",
                    "metric": "all_smi_gpu_power_consumption_watts", "labels": {"gpu_index": "0"},
                    "unit": "W", "value": 300.0})
        for lim in ("all_smi_gpu_power_limit_current_watts", "all_smi_gpu_power_limit_max_watts"):
            out.append({"kind": "telemetry", "run_id": "r", "t_ns": t, "src": "all-smi",
                        "metric": lim, "labels": {"gpu_index": "0"}, "unit": "W", "value": 350.0})
        mj = 1_000_000.0 + i * 300_000.0  # 300 W = 300,000 mJ per second
        e = {"kind": "telemetry", "run_id": "r", "t_ns": t, "src": "all-smi",
             "metric": "all_smi_gpu_energy_hw_millijoules_total", "labels": {"gpu_index": "0"},
             "unit": "J", "value": mj * 0.001 if scaled else mj}
        if scaled:
            e["raw"] = mj
        out.append(e)
    out.append({"kind": "request", "run_id": "r", "stage": 1.0, "success": True, "output_tokens": 1000})
    return out
for name, scaled in (("raw", False), ("scaled", True)):
    (w / f"{name}.ndjson").write_text("\n".join(json.dumps(r) for r in rows(scaled)) + "\n")
PY

fail=0
for name in raw scaled; do
  line="$(python3 "${ROOT}/docs/queries/analyze.py" "${work}/${name}.ndjson" | awk -F'\t' '$1=="1.0"')"
  read -r _stage _load _n mean _p95 counter trap out jtok <<<"$(tr '\t' ' ' <<<"${line}")"
  # Stage window [0 s, 10 s) holds samples 0..9 s: 300 W over 9 s is 2700 J
  # from both the counter and the trapezoid, 2.7 J per output token.
  python3 - "${name}" "${mean}" "${counter}" "${trap}" "${jtok}" <<'PY' || fail=1
import sys
name, mean, counter, trap, jtok = sys.argv[1], *map(float, sys.argv[2:])
ok = abs(mean - 300) < 1e-6 and abs(counter - 2700) < 1e-6 and abs(trap - 2700) < 1e-6 and abs(jtok - 2.7) < 1e-9
print(f"analyze_test {name}: power_mean={mean} counter_j={counter} trap_j={trap} j_per_tok={jtok} {'ok' if ok else 'FAIL'}")
sys.exit(0 if ok else 1)
PY
done
[[ "${fail}" -eq 0 ]] || { echo "analyze_test: FAIL"; exit 1; }
echo "analyze_test: ok"
