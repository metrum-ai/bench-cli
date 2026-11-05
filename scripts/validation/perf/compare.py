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
#              [--noise-floor noise-floor.json] \
#              [--tolerance-pct 5] [--out-md table.md] [--out-json table.json]
#
# --noise-floor loads per-point, per-metric repeat-noise floors
# {"<model>": {"W1": {"c1": {"system_output_tok_s": 0.58, ...}}}}; a point's
# gate tolerance is max(--tolerance-pct, its noise floor) per metric. A point
# directory missing from either root is a FAIL verdict ("missing"), because a
# run that did not produce the full grid is not a valid PASS.
import argparse, csv, datetime, json, math, pathlib, statistics, sys


def pct(a, p):
    a = sorted(a)
    if not a:
        return None
    x = (len(a) - 1) * p
    i = int(x)
    return a[i] + (a[min(i + 1, len(a) - 1)] - a[i]) * (x - i)


def iso(s):
    value = datetime.datetime.fromisoformat(s.strip().replace('Z', '+00:00'))
    if value.tzinfo is None:
        raise ValueError('request timestamp lacks timezone')
    return value.timestamp()


def legacy_window(rows, successes):
    if not rows or not successes:
        raise ValueError('legacy comparison needs measured and successful requests')
    offsets = [row.get('send_offset_s') for row in rows]
    if all(isinstance(v, (int, float)) and not isinstance(v, bool) and math.isfinite(v) for v in offsets):
        starts = offsets
    elif all(v is None for v in offsets):
        starts = [iso(row['started_at']) for row in rows]
    else:
        raise ValueError('mixed or invalid send-clock sources')
    sends = {id(row): start for row, start in zip(rows, starts)}
    ends = []
    for row in successes:
        latency = row.get('latency_s')
        if not isinstance(latency, (int, float)) or isinstance(latency, bool) or not math.isfinite(latency) or latency <= 0:
            raise ValueError('invalid successful request latency')
        ends.append(sends[id(row)] + latency)
    window = max(ends) - min(starts)
    if not math.isfinite(window) or window <= 0:
        raise ValueError('invalid legacy success window')
    return window


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
    with (folder / 'requests.jsonl').open(encoding='utf-8') as stream:
        rows = [json.loads(line) for line in stream if line.strip()]
    rows, summary = select_run(rows)
    if summary is None or not (folder / 'DONE').is_file():
        raise ValueError('complete summary and successful process marker required')
    rows = [x for x in rows if x.get("phase") == "measure"]
    ok = [x for x in rows if x.get('error') is None and x.get('outcome') != 'reasoning_truncated']
    if any(row.get('first_reasoning_s') is not None for row in ok):
        raise ValueError('reasoning observed in the thinking-off legacy comparison')
    dur = legacy_window(rows, ok)
    r = {"workload": meta["workload"], "concurrency": meta["concurrency"],
         "sample_count": len(ok), "tool_wall_time_s": dur,
         "error_rate": (len(rows) - len(ok)) / max(len(rows), 1),
         "requests_s": len(ok) / dur if dur else None,
         "window_definition": "first measured send to last legacy-success terminal",
         "headline": ({key: summary.get(key) for key in ('attempted', 'successes', 'errors', 'ttft_coverage',
                       'window_seconds', 'completion_tokens_per_second', 'completion_tokens_total', 'prompt_tokens_total',
                       'usage_missing_count', 'ttft_first_any_s', 'legacy_success_completion_tokens_per_second')} if summary else None)}
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
        for x in csv.DictReader(power.open(), skipinitialspace=True):
            try:
                vals.append(float((x.get("power.draw [W]") or x["power.draw"]).split()[0]))
            except (ValueError, KeyError, IndexError):
                pass
        r["power_mean_w"] = statistics.mean(vals) if vals else None
    return r


def rel_delta(a, b):
    if a is None or b is None or b == 0:
        return None
    return 100.0 * (a - b) / abs(b)


def load_noise_floor(path, model):
    """{model: {W: {cN: {metric: pct}}}} -> {(W, c): {metric: pct}} for `model`."""
    if not path:
        return {}
    with open(path, encoding='utf-8') as stream:
        data = json.load(stream)
    out = {}
    for w, cc_map in (data.get(model) or {}).items():
        for c_key, metrics in (cc_map or {}).items():
            try:
                c = int(str(c_key).lstrip("c"))
            except ValueError:
                raise ValueError(f"noise floor: bad concurrency key {c_key!r} under {model}.{w}")
            parsed = {k: float(v) for k, v in (metrics or {}).items()}
            if any(not math.isfinite(v) or v < 0 for v in parsed.values()):
                raise ValueError('noise floors must be finite and non-negative')
            out[(w, c)] = parsed
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--baseline-root", required=True)
    ap.add_argument("--candidate-root", required=True)
    ap.add_argument("--model", required=True)
    ap.add_argument("--pass", dest="pass_", default="1")
    ap.add_argument("--reference", help="optional study reference JSON (per-point scalars)")
    ap.add_argument("--noise-floor", help="per-point repeat-noise floors; tolerance is max(tolerance-pct, floor)")
    ap.add_argument("--tolerance-pct", type=float, default=5.0)
    ap.add_argument("--out-md")
    ap.add_argument("--out-json")
    a = ap.parse_args()

    ref = {}
    if a.reference:
        for p in json.load(open(a.reference)):
            ref[(p["workload"], int(p["concurrency"]))] = p
    try:
        noise = load_noise_floor(a.noise_floor, a.model)
    except (ValueError, OSError, json.JSONDecodeError) as e:
        sys.exit(f"compare.py: bad --noise-floor file: {e}")

    def tolerance_for(metric, w, c):
        return max(a.tolerance_pct, (noise.get((w, c)) or {}).get(metric, 0.0))

    rows = []
    missing = []
    verdict = "PASS"
    for w in ("W1", "W2"):
        for c in (1, 8, 32):
            b_dir = pathlib.Path(a.baseline_root) / a.pass_ / w / f"c{c}"
            k_dir = pathlib.Path(a.candidate_root) / a.pass_ / w / f"c{c}"
            absent = [label for label, d in (("baseline", b_dir), ("candidate", k_dir))
                      if not (d / "point.json").exists()]
            if absent:
                missing.append(f"{w}/c{c}: {' and '.join(absent)}")
                verdict = "FAIL"
                rows.append({"workload": w, "concurrency": c, "missing": absent,
                             "baseline": None, "candidate": None,
                             "checks": {m: {"delta_pct": None, "pass": False} for m in
                                        ("system_output_tok_s", "tpot_p50_ms")}})
                continue
            b = extract_point(b_dir)
            k = extract_point(k_dir)
            if any(point['workload'] != w or point['concurrency'] != c for point in (b, k)):
                raise ValueError('point metadata does not match the frozen workload and concurrency')
            row = {"workload": w, "concurrency": c, "baseline": b, "candidate": k, "checks": {}}
            for m in ("system_output_tok_s", "tpot_p50_ms"):
                d = rel_delta(k.get(m), b.get(m))
                okm = d is not None and abs(d) <= tolerance_for(m, w, c)
                row["checks"][m] = {"delta_pct": d, "pass": okm,
                                    "tolerance_pct": tolerance_for(m, w, c)}
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
        if row.get("missing"):
            lines.append(f"| {row['workload']} c{row['concurrency']} | — | — | — | — | — | — | — | "
                         f"FAIL (missing: {' + '.join(row['missing'])}) |")
            continue
        b, k, ch = row["baseline"], row["candidate"], row["checks"]
        ok = all(x["pass"] for x in ch.values())
        lines.append(
            f"| {row['workload']} c{row['concurrency']} | {f(b.get('system_output_tok_s'))} | "
            f"{f(k.get('system_output_tok_s'))} | {f(ch['system_output_tok_s']['delta_pct'])} | "
            f"{f(b.get('tpot_p50_ms'))} | {f(k.get('tpot_p50_ms'))} | {f(ch['tpot_p50_ms']['delta_pct'])} | "
            f"{f(b.get('ttft_p50_ms'))} / {f(k.get('ttft_p50_ms'))} | {'PASS' if ok else 'FAIL'} |")
    lines += ["", "Candidate native headline (informational; not the legacy-cohort gate):", "",
              "| point | headline window s | headline completion tok/s | first-any TTFT p50 ms |",
              "|---|---:|---:|---:|"]
    for row in rows:
        if not row.get('candidate'):
            continue
        headline = row['candidate'].get('headline') or {}
        first_any = headline.get('ttft_first_any_s') or {}
        first_any_p50 = first_any.get('p50') if isinstance(first_any, dict) else None
        lines.append(f"| {row['workload']} c{row['concurrency']} | {f(headline.get('window_seconds'))} | "
                     f"{f(headline.get('completion_tokens_per_second'))} | "
                     f"{f(first_any_p50 * 1000 if first_any_p50 is not None else None)} |")
    gate_note = f"gate: output tok/s and TPOT p50 within {a.tolerance_pct}%"
    if noise:
        floors = sorted({tol for (_, _), m in noise.items() for tol in m.values()})
        gate_note += f" (per-point noise floor up to {max(floors)}%, whichever is wider)"
    gate_note += "; TTFT informational, #313 caveats"
    lines += ["", f"**Overall: {verdict}** ({gate_note})", ""]
    if missing:
        lines += [f"Missing points ({len(missing)}): " + "; ".join(missing), ""]
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
