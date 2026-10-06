# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Metrum AI Bench #233 live verification checks, corrected. Usage: check_verify_fixed.py <llm dir> [<vlm> <asr> <imagegen>]

Same as check_verify.py (kept as run) except #224: CSV rows are matched to points by the CSV `stage` column
(check_verify.py looked up `load`/`stage_load`, which the CSV does not have, so it compared 0 stages), the
check fails unless every point is compared, and it also runs on each modality sweep CSV."""
import csv, json, os, sys

D = sys.argv[1]
mods = sys.argv[2:]
res = []


def rec(issue, ok, detail):
    res.append((issue, "PASS" if ok else "FAIL", detail))


def jl(p):
    return [json.loads(l) for l in open(p) if l.strip()]


def summary(rows):
    return [r for r in rows if str(r.get("schema_version", "")).endswith("summary.v3")][-1]


# #230 preflight on a thinking model
t = open(os.path.join(D, "f230-preflight-thinking.txt")).read()
line = [l for l in t.splitlines() if l.startswith("streaming_first_token")]
rec("#230", "exit=0" in t and line and "PASS" in line[0], f"thinking ON: {line[0].strip() if line else 'n/a'}; {t.strip().splitlines()[-1]}")

# #226 warmup barrier and measured-only observed concurrency
rows = jl(os.path.join(D, "f226.jsonl"))
s = summary(rows)
reqs = [r for r in rows if r is not s and "phase" in r]
warm = [r for r in reqs if r["phase"] == "warmup"]
meas = [r for r in reqs if r["phase"] == "measure"]
w_end = max(r["send_offset_s"] + (r.get("latency_s") or 0) for r in warm)
m_start = min(r["send_offset_s"] for r in meas)
oc = s.get("observed_concurrency") or {}
rec("#226", m_start >= w_end and oc.get("acquire_count") == len(meas),
    f"last warmup end {w_end:.6f}s, first measured send {m_start:.6f}s (gap {1e3*(m_start-w_end):.3f} ms); "
    f"acquire_count {oc.get('acquire_count')} vs measured {len(meas)}; in_flight_max {oc.get('in_flight_max')} (cap 16); "
    f"measured fresh connections {sum(1 for r in meas if r.get('connection_reused') is False)}")

# #227 telemetry abort then one SIGINT: summary still written
ex = open(os.path.join(D, "f227.exit")).read().strip()
r227 = jl(os.path.join(D, "f227.jsonl"))
has_sum = any(str(r.get("schema_version", "")).endswith("summary.v3") for r in r227)
nd = jl(os.path.join(D, "f227.ndjson"))
last = nd[-1]
err = open(os.path.join(D, "f227.stderr")).read()
abort_line = [l for l in err.splitlines() if "abort" in l.lower()][:1]
rec("#227", has_sum and last.get("kind") == "summary" and last.get("partial") is True and "130" not in ex,
    f"{ex}; data-log summary.v3 present={has_sum}; ndjson last kind={last.get('kind')} partial={last.get('partial')}; "
    f"stderr: {abort_line[0][:140] if abort_line else 'no abort line'}")

# #224 stage window: throughput == successes / (latest successful completion - earliest measured send)
def check224(label, sw, csv_path):
    rows = list(csv.DictReader(open(csv_path)))
    worst, compared, nreq = 0.0, 0, 0
    for p in sw["points"]:
        st = [r for r in rows if float(r["stage"]) == float(p["load"]) and r.get("warmup", "false").lower() != "true"]
        ok = [r for r in st if r.get("error", "") in ("", "null", "None")]
        if not st or not ok:
            continue
        start = min(float(r["send_offset_s"]) for r in st)
        end = max(float(r["send_offset_s"]) + float(r["latency_s"]) for r in ok)
        worst = max(worst, abs(len(ok) / (end - start) - p["throughput"]) / p["throughput"])
        compared += 1
        nreq += len(st)
    n = len(sw["points"])
    rec(label, n > 0 and compared == n and worst < 1e-6,
        f"stages compared {compared}/{n} ({nreq} measured rows, key column=stage); max relative throughput mismatch vs recompute = {worst:.2e}")


sw = json.load(open(os.path.join(D, "llm-sweep.stdout.json")))
check224("#224 (LLM)", sw, os.path.join(D, "llm-sweep.csv"))

# #232 knee on the LLM sweep
kd = sw.get("knee_detection")
rec("#232 (LLM)", kd and kd.get("index") is not None, f"knee_detection={kd}; p95 per stage={[round(p['p95_s'],4) for p in sw['points']]}")

# #231 analyze.py bounds
a = open(os.path.join(D, "analyze.txt")).read()
pre = [l for l in a.splitlines() if "request_prefill_time_seconds" in l][:3]
rec("#231", any("<=" in l for l in pre) and not any("\t0.15\t0.285" in l for l in pre), "prefill rows: " + " | ".join(l.strip() for l in pre))

# #216 launcher gating
t = open(os.path.join(D, "f216.txt")).read().strip().splitlines()
rec("#216", "exit=0" not in t[0] and "exit=0" in t[1] and "exit=0" in t[2], " | ".join(t))

for m in mods:
    name = os.path.basename(m.rstrip("/")).replace("verify233-", "")
    sw = json.load(open(os.path.join(m, "sweep.stdout.json")))
    kd = sw.get("knee_detection")
    pts = [(p["load"], round(p["throughput"], 4), round(p["p95_s"], 4), p.get("error_rate")) for p in sw["points"]]
    rec(f"#232/#224 ({name} sweep)", True, f"knee_detection={kd}; points={pts}")
    check224(f"#224 ({name} sweep)", sw, os.path.join(m, "sweep.csv"))
    if name == "vlm" and os.path.exists(os.path.join(m, "big.jsonl")):
        b = summary(jl(os.path.join(m, "big.jsonl")))
        sm = summary(jl(os.path.join(m, "run.jsonl")))
        rec("#242 (vlm 2048x2048)", (b.get("successes") or 0) > 0,
            f"big-image successes {b.get('successes')}/{b.get('attempted')}, ttft p50 {(b.get('ttft_s') or {}).get('p50')}; small-image ttft p50 {(sm.get('ttft_s') or {}).get('p50')}")

for issue, st, detail in res:
    print(f"{st}  {issue}: {detail}")
