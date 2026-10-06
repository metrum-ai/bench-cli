# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Metrum AI Bench epic #184 live checks for the VLM/ASR/imagegen lanes (#196 telemetry, #197 sweeps)."""
import json, os, sys

L = sys.argv[1]
for m in ("vlm", "asr", "imagegen"):
    D = os.path.join(L, f"epic184-{m}")
    rows = [json.loads(l) for l in open(os.path.join(D, "run.jsonl")) if l.strip()]
    s = [r for r in rows if r.get("schema_version", "").endswith("summary.v3")][-1]
    reqs = [r for r in rows if not r.get("schema_version", "").endswith("summary.v3")]
    nd = [json.loads(l) for l in open(os.path.join(D, "run.ndjson")) if l.strip()]
    kinds = {}
    for r in nd:
        kinds[r["kind"]] = kinds.get(r["kind"], 0) + 1
    nreq = {r["seq"]: r for r in nd if r.get("kind") == "request"}
    diffs = [abs(r["send_offset_s"] * 1e9 - nreq[r["seq"]]["t_sent_ns"]) for r in reqs
             if r.get("seq") in nreq and r.get("send_offset_s") is not None]
    metrics = sorted({r["metric"] for r in nd if r.get("kind") == "telemetry"})
    eng = [x for x in metrics if not x.startswith("all_smi")]
    print(f"\n=== {m}: attempted={s.get('attempted')} successes={s.get('successes')} errors={s.get('errors_by_type')}")
    print(f"  #196 telemetry={s.get('telemetry')}")
    print(f"  #196 ndjson kinds={kinds} first={nd[0]['kind']} join max|diff|ns={max(diffs) if diffs else None} engine series={len(eng)} e.g. {eng[:3]}")
    print(f"  #194 reuse={s.get('connection_reuse_rate')} connect n={((s.get('connect_s') or {}).get('n'))}  #195 eff_conc avg={(s.get('effective_concurrency') or {}).get('avg')}")
    mm = s.get("modality_metrics") or {}
    print(f"  modality metrics keys: {sorted(mm)[:10] if isinstance(mm, dict) else mm}")
    sw = json.load(open(os.path.join(D, "sweep.stdout.json")))
    print(f"  #197 sweep kind={sw.get('config', {}).get('kind')} knee_detection={sw.get('knee_detection')}")
    for p in sw.get("points", []):
        pm = p.get("modality_metrics") or {}
        dig = p.get("image_digests")
        print(f"    load={p.get('load')} n={p.get('n')} err={p.get('error_rate')} thr={p.get('throughput')} p95={p.get('p95_s')} "
              f"mm={ {k: (v.get('avg') if isinstance(v, dict) else v) for k, v in list(pm.items())[:3]} } digests={len(dig) if isinstance(dig, list) else dig}")
    swnd = [json.loads(l) for l in open(os.path.join(D, "sweep.ndjson")) if l.strip()]
    sk = {}
    for r in swnd:
        sk[r["kind"]] = sk.get(r["kind"], 0) + 1
    last = swnd[-1]
    print(f"  #197 sweep ndjson kinds={sk} last={last['kind']} partial={last.get('partial')} run.config.kind={swnd[0].get('config', {}).get('kind')} modality keys in run row={[k for k in swnd[0].get('config', {}) if k in ('modality','temperature')]}")
