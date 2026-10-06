# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Metrum AI Bench epic #184 live checks on the LLM lane outputs (run locally on fetched files)."""
import json, sys, os

D = sys.argv[1]
ok = lambda c: "PASS" if c else "FAIL"


def load(path):
    return [json.loads(l) for l in open(os.path.join(D, path)) if l.strip()]


def split(rows):
    summ = [r for r in rows if r.get("schema_version", "").endswith("summary.v3")]
    reqs = [r for r in rows if not r.get("schema_version", "").endswith("summary.v3")]
    return reqs, (summ[-1] if summ else {})


def dist(s, k):
    v = s.get(k) or {}
    return v if isinstance(v, dict) else {}


for run, cap in (("r1-plain", 16), ("r2-reasoning", 8)):
    reqs, s = split(load(run + ".jsonl"))
    meas = [r for r in reqs if r.get("phase", "measure") == "measure"]
    succ = [r for r in meas if not r.get("error")]
    print(f"\n=== {run}: {len(meas)} measured, {len(succ)} ok")
    oc = s.get("observed_concurrency") or {}
    print(f"#189 in_flight_max={oc.get('in_flight_max')} mean={oc.get('in_flight_mean')} cap={cap}: {ok((oc.get('in_flight_max') or 99) <= cap)}")
    present = [k for k in ("first_byte_s", "queue_delay_s", "first_reasoning_s", "isl_tokens", "osl_tokens") if k in s]
    print(f"#191 new distributions present: {present} n=" + str({k: dist(s, k).get('n') for k in present}))
    rt = [r.get("reasoning_tokens") for r in succ]
    print(f"#192 reasoning_tokens non-null {sum(x is not None for x in rt)}/{len(rt)}; total={s.get('reasoning_tokens_total')}")
    bad = [r for r in succ if r.get("reasoning_tokens") is not None and r.get("visible_completion_tokens") is not None
           and r["reasoning_tokens"] + r["visible_completion_tokens"] != r.get("completion_tokens")]
    print(f"#192 reasoning+visible==completion: {ok(not bad)} ({len(bad)} mismatches)")
    print(f"#193 prompt_tokens_total={s.get('prompt_tokens_total')} completion_tokens_total={s.get('completion_tokens_total')} "
          f"input_tps={s.get('input_tokens_per_second')} total_tps={s.get('total_tokens_per_second')} "
          f"prefill_tps_per_user p50={dist(s,'prefill_tps_per_user').get('p50')} tt2t p50={dist(s,'time_to_second_token_s').get('p50')} user_tps p50={dist(s,'user_tps').get('p50')}")
    w = s.get("window_seconds") or 0
    if w and s.get("completion_tokens_total"):
        print(f"#193 completion_tokens_total/window = {s['completion_tokens_total']/w:.3f} vs completion_tokens_per_second {s.get('completion_tokens_per_second')}")
    reused = [r.get("connection_reused") for r in meas]
    fresh = sum(1 for x in reused if x is False)
    print(f"#194 connection_reuse_rate={s.get('connection_reuse_rate')} fresh_connections_measured={fresh} (expect <= {cap}): {ok(fresh <= cap)}; "
          f"dns_s n={dist(s,'dns_s').get('n')} bytes_received p50={dist(s,'bytes_received').get('p50')} chunks p50={dist(s,'chunks_received').get('p50')}")
    ec = s.get("effective_concurrency") or {}
    print(f"#195 effective_concurrency avg={ec.get('avg')} max={ec.get('max')} (<= {cap}): {ok((ec.get('max') or 99) <= cap)}; "
          f"decode_tput avg={(s.get('effective_decode_throughput') or {}).get('avg')} tokens_in_flight max={(s.get('tokens_in_flight') or {}).get('max')}")
    lat = [r["latency_s"] for r in succ if r.get("latency_s") is not None]
    if w and lat:
        print(f"#195 Little's law: sum(latency)/window = {sum(lat)/w:.4f} vs effective_concurrency.avg {ec.get('avg')}")
    tel = s.get("telemetry")
    print(f"#196 summary.telemetry: {tel}")
    nd = load(run + ".ndjson")
    kinds = {}
    for r in nd:
        kinds[r.get("kind")] = kinds.get(r.get("kind"), 0) + 1
    print(f"#196 ndjson kinds: {kinds}; first row kind={nd[0].get('kind')}")
    nreq = {r["seq"]: r for r in nd if r.get("kind") == "request" and "seq" in r}
    diffs = [abs(r["send_offset_s"] * 1e9 - nreq[r["seq"]]["t_sent_ns"]) for r in reqs
             if r.get("seq") in nreq and r.get("send_offset_s") is not None and nreq[r["seq"]].get("t_sent_ns") is not None]
    print(f"#196 data-log send_offset_s vs ndjson t_sent_ns: max |diff| = {max(diffs) if diffs else None} ns over {len(diffs)} rows: {ok(diffs and max(diffs) < 1000)}")
    tele = [r for r in nd if r.get("kind") == "telemetry"]
    ts = [r["t_ns"] for r in tele if "t_ns" in r]
    names = sorted({r.get("name") for r in tele})
    lo = min(r["t_sent_ns"] for r in nreq.values() if r.get("t_sent_ns") is not None)
    hi = max(r["t_done_ns"] for r in nreq.values() if r.get("t_done_ns") is not None)
    print(f"#196 telemetry samples={len(tele)} series_names={len(names)} t_ns range=[{min(ts) if ts else None},{max(ts) if ts else None}] request window=[{lo},{hi}]")
    print(f"#198 vllm histogram names present: {[n for n in names if n and 'vllm:' in n and n.endswith('_bucket')][:6]}")
    print(f"#198 per-core cpu series present: {[n for n in names if n and 'cpu_core' in n][:2]}; process series: {[n for n in names if n and 'process' in n][:2]}")
    if run == "r2-reasoning":
        pt = dist(s, "prefill_tps_per_user").get("p50")
        fr = dist(s, "first_reasoning_s").get("p50")
        print(f"#193 reasoning prefill_tps_per_user p50={pt} first_reasoning_s p50={fr} ttft p50={dist(s,'ttft_s').get('p50')}")

print("\n=== r3 sweep")
sw = json.load(open(os.path.join(D, "r3-sweep.stdout.json")))
pts = sw.get("points") or sw.get("sweep") or []
print("#190 knee:", (sw.get("knee") or {}).get("load") if isinstance(sw.get("knee"), dict) else sw.get("knee"), "knee_detection:", sw.get("knee_detection"))
for p in pts:
    print(f"  load={p.get('load')} n={p.get('n')} thr={p.get('throughput_rps')} p95={p.get('p95_s')} err={p.get('error_rate')} "
          f"eff_conc={(p.get('effective_concurrency') or {}).get('avg')} reuse={p.get('connection_reuse_rate')} "
          f"tt2t={(p.get('time_to_second_token_s') or {}).get('p50') if isinstance(p.get('time_to_second_token_s'), dict) else p.get('time_to_second_token_s')}")
