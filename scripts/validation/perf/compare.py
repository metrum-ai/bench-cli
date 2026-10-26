#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Perf-validation compare (issue #333). Reads runner.sh output roots for the
# baseline and candidate builds, derives the per-point scalars from the raw
# request logs, and emits a per-point PASS/FAIL table (markdown + JSON).
# Gates per spec.yaml: system_output_tok_s and tpot_p50_ms within tolerance
# (default 5%, or the point's noise floor if wider). TTFT is reported with the
# #313 cohort caveats and is never a gate. Optionally diffs both builds against
# a reference (study) JSON as informational box/day drift.
#
# Usage:
#   compare.py --baseline-root RUNS/q38-27b/bench-rel \
#              --candidate-root RUNS/q38-27b/bench-src \
#              --model q38-27b --pass 1 \
#              [--reference study-reference.json] \
#              [--tolerance-pct 5] [--out-md table.md] [--out-json table.json]
import argparse, csv, json, pathlib, statistics, sys


def pct(a, p):
    a = sorted(a)
    if not a:
        return None
    x = (len(a) - 1) * p
    i = int(x)
    return a[i] + (a[min(i + 1, len(a) - 1)] - a[i]) * (x - i)


def iso(s):
    import datetime
    s = s.strip()
    if "." in s:
        head, tail = s.split(".", 1)
        frac = "".join(c for c in tail if c.isdigit())[:6]
        s = head + "." + frac
    return datetime.datetime.fromisoformat(s.replace("Z", "+00:00")).timestamp()


def select_run(records):
    summaries = [x for x in records if ".summary." in x.get("schema_version", "")]
    requests = [x for x in records if ".request." in x.get("schema_version", "")]
    complete = [x for x in summaries if not x.get("partial", False)]
    if len(complete) > 1:
        raise ValueError("multiple complete summaries; isolate the intended run")
    if summaries and not complete:
        raise ValueError("only partial summaries; no complete run")
    summary = complete[0] if complete else None
    run_id = (summary.get("config") or {}).get("run_id") or summary.get("run_id") if summary else None
    if run_id:
        selected = [x for x in requests if x.get("run_id") == run_id]
        if not selected:
            raise ValueError("complete summary has no matching request run_id")
        return selected, summary
    ids = {x.get("run_id") for x in requests}
    if len(ids) > 1:
        raise ValueError("multiple request run_ids without a selectable summary")
    return requests, summary


def extract_point(folder):
    folder = pathlib.Path(folder)
    meta = json.loads((folder / "point.json").read_text())
    rows = [json.loads(l) for l in (folder / "requests.jsonl").open()]
    rows, summary = select_run(rows)
    rows = [x for x in rows if x.get("phase") == "measure"]
    ok = [x for x in rows if not x.get("error")]
    dur = None
    if summary and summary.get("window_seconds") is not None:
        dur = summary["window_seconds"]
    if dur is None and ok:
        dur = max(iso(x["completed_at"]) for x in ok) - min(iso(x["started_at"]) for x in rows)
    r = {"workload": meta["workload"], "concurrency": meta["concurrency"],
         "sample_count": len(ok), "tool_wall_time_s": dur,
         "error_rate": (len(rows) - len(ok)) / max(len(rows), 1),
         "requests_s": len(ok) / dur if dur else None}
    for metric, ps in [("ttft", [50, 90]), ("tpot", [50, 90])]:
        vals = []
        for x in ok:
            if metric == "tpot":
                n = x.get("completion_tokens", 0)
                t = x.get("ttft_s")
                v = max(x["latency_s"] - t, 0) / (n - 1) if n > 1 and t is not None and x["latency_s"] > t else None
                if v is not None:
                    vals.append(v)
            elif x.get("ttft_s") is not None:
                vals.append(x["ttft_s"])
        for p in ps:
            v = pct(vals, p / 100)
            r[f"{metric}_p{p}_ms"] = v * 1000 if v is not None else None
    valid = [x for x in ok if not x.get("usage_missing")]
    if valid and len(valid) == len(ok):
        inp = sum(x["prompt_tokens"] for x in valid)
        out = sum(x["completion_tokens"] for x in valid)
        r.update(mean_input_tokens=inp / len(valid), mean_output_tokens=out / len(valid),
                 output_tokens=out, system_output_tok_s=out / dur if dur else None)
    power = folder / "nvidia.csv"
    if power.exists():
        vals = []
        for x in csv.DictReader(power.open()):
            try:
                vals.append(float(x["power.draw"].split()[0]))
            except (ValueError, KeyError, IndexError):
                pass
        r["power_mean_w"] = statistics.mean(vals) if vals else None
    return r


def rel_delta(a, b):
    if a is None or b is None or b == 0:
        return None
    return 100.0 * (a - b) / abs(b)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--baseline-root", required=True)
    ap.add_argument("--candidate-root", required=True)
    ap.add_argument("--model", required=True)
    ap.add_argument("--pass", dest="pass_", default="1")
    ap.add_argument("--reference", help="optional study reference JSON (per-point scalars)")
    ap.add_argument("--tolerance-pct", type=float, default=5.0)
    ap.add_argument("--out-md")
    ap.add_argument("--out-json")
    a = ap.parse_args()

    ref = {}
    if a.reference:
        for p in json.load(open(a.reference)):
            ref[(p["workload"], int(p["concurrency"]))] = p

    rows = []
    verdict = "PASS"
    for w in ("W1", "W2"):
        for c in (1, 8, 32):
            b = extract_point(pathlib.Path(a.baseline_root) / a.pass_ / w / f"c{c}")
            k = extract_point(pathlib.Path(a.candidate_root) / a.pass_ / w / f"c{c}")
            row = {"workload": w, "concurrency": c, "baseline": b, "candidate": k, "checks": {}}
            for m in ("system_output_tok_s", "tpot_p50_ms"):
                d = rel_delta(k.get(m), b.get(m))
                okm = d is not None and abs(d) <= a.tolerance_pct
                row["checks"][m] = {"delta_pct": d, "pass": okm}
                if not okm:
                    verdict = "FAIL"
            if (w, c) in ref:
                s = ref[(w, c)]
                row["study"] = {m: {"value": s.get(m),
                                    "baseline_delta_pct": rel_delta(b.get(m), s.get(m)),
                                    "candidate_delta_pct": rel_delta(k.get(m), s.get(m))}
                                for m in ("system_output_tok_s", "tpot_p50_ms", "ttft_p50_ms",
                                          "requests_s", "power_mean_w")}
            rows.append(row)

    def f(v, nd=2):
        return "null" if v is None else f"{v:.{nd}f}"

    lines = [f"## Perf validation: {a.model} (pass {a.pass_})", "",
             "| point | base tok/s | cand tok/s | Δ% | base TPOT50 ms | cand TPOT50 ms | Δ% | TTFT50 ms base/cand (info) | verdict |",
             "|---|---:|---:|---:|---:|---:|---:|---|:---:|"]
    for row in rows:
        b, k, ch = row["baseline"], row["candidate"], row["checks"]
        ok = all(x["pass"] for x in ch.values())
        lines.append(
            f"| {row['workload']} c{row['concurrency']} | {f(b.get('system_output_tok_s'))} | "
            f"{f(k.get('system_output_tok_s'))} | {f(ch['system_output_tok_s']['delta_pct'])} | "
            f"{f(b.get('tpot_p50_ms'))} | {f(k.get('tpot_p50_ms'))} | {f(ch['tpot_p50_ms']['delta_pct'])} | "
            f"{f(b.get('ttft_p50_ms'))} / {f(k.get('ttft_p50_ms'))} | {'PASS' if ok else 'FAIL'} |")
    lines += ["", f"**Overall: {verdict}** (gate: output tok/s and TPOT p50 within {a.tolerance_pct}%; "
                  "TTFT informational, #313 caveats)", ""]
    md = "\n".join(lines)
    out = {"model": a.model, "pass": a.pass_, "tolerance_pct": a.tolerance_pct,
           "verdict": verdict, "points": rows}
    if a.out_md:
        pathlib.Path(a.out_md).write_text(md)
    if a.out_json:
        pathlib.Path(a.out_json).write_text(json.dumps(out, indent=1))
    print(md)
    sys.exit(0 if verdict == "PASS" else 1)


if __name__ == "__main__":
    main()
