# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Metrum AI Bench #224 stage-window recompute from a strategic sweep CSV (keys stages by the CSV `stage` column).

Usage: recompute_224.py <dir with sweep.stdout.json and sweep.csv> [<stdout.json> <csv>]
throughput == successes / (latest successful completion - earliest measured send), from send_offset_s + latency_s.
"""
import csv, json, os, sys

d = sys.argv[1]
js = sys.argv[2] if len(sys.argv) > 2 else os.path.join(d, "sweep.stdout.json")
cv = sys.argv[3] if len(sys.argv) > 3 else os.path.join(d, "sweep.csv")
sw = json.load(open(js))
rows = list(csv.DictReader(open(cv)))
worst = 0.0
print(f"# {os.path.basename(d.rstrip('/'))}: {len(rows)} csv rows, {len(sw['points'])} points")
for p in sw["points"]:
    st = [r for r in rows if float(r["stage"]) == float(p["load"]) and r["warmup"].lower() != "true"]
    ok = [r for r in st if r["success"].lower() == "true" and not r["error"]]
    start = min(float(r["send_offset_s"]) for r in st)
    end = max(float(r["send_offset_s"]) + float(r["latency_s"]) for r in ok)
    thr = len(ok) / (end - start)
    rel = abs(thr - p["throughput"]) / p["throughput"]
    worst = max(worst, rel)
    print(f"load={p['load']:g} measured={len(st)} ok={len(ok)} window_s={end - start:.6f} "
          f"recomputed={thr:.6f} reported={p['throughput']:.6f} rel_diff={rel:.2e}")
print(f"{'PASS' if worst < 1e-6 else 'FAIL'}  #224: max relative mismatch {worst:.2e}")
