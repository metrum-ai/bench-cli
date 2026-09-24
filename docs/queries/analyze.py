#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Stdlib analysis of strategic telemetry NDJSON (DuckDB optional elsewhere)."""

from __future__ import annotations

import json
import math
import sys
from collections import defaultdict
from typing import Any


POWER_METRICS = (
    "DCGM_FI_DEV_POWER_USAGE",
    "all_smi_gpu_power_consumption_watts",
    "nvidia_smi_power_draw_watts",
    "gpu_power_usage",
    "gpu_package_power",
)
ENERGY_METRICS = (
    "DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION",
    "all_smi_energy_consumed_joules_total",
)


def load_rows(path: str) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            rows.append(json.loads(line))
    return rows


def trapezoid_energy(samples: list[tuple[int, float]]) -> float:
    if len(samples) < 2:
        return 0.0
    samples = sorted(samples)
    joules = 0.0
    for (t0, p0), (t1, p1) in zip(samples, samples[1:]):
        dt = (t1 - t0) / 1e9
        joules += 0.5 * (p0 + p1) * dt
    return joules


def mean(xs: list[float]) -> float:
    return sum(xs) / len(xs) if xs else float("nan")


def percentile(xs: list[float], p: float) -> float:
    if not xs:
        return float("nan")
    ys = sorted(xs)
    if len(ys) == 1:
        return ys[0]
    k = (len(ys) - 1) * (p / 100.0)
    f = math.floor(k)
    c = math.ceil(k)
    if f == c:
        return ys[int(k)]
    return ys[f] * (c - k) + ys[c] * (k - f)


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {sys.argv[0]} run.ndjson", file=sys.stderr)
        return 2
    rows = load_rows(sys.argv[1])
    by_kind: dict[str, int] = defaultdict(int)
    for r in rows:
        by_kind[r.get("kind", "?")] += 1
    print("## row counts")
    for k, n in sorted(by_kind.items()):
        print(f"{k}: {n}")

    stages = [r for r in rows if r.get("kind") == "stage" and r.get("phase") == "measure"]
    tele = [r for r in rows if r.get("kind") == "telemetry"]
    reqs = [r for r in rows if r.get("kind") == "request" and not r.get("warmup")]

    by_src: dict[str, int] = defaultdict(int)
    for t in tele:
        by_src[t.get("src", "?")] += 1
    print("\n## telemetry by src")
    for s, n in sorted(by_src.items()):
        print(f"{s}: {n}")

    print("\n## measured stages")
    print(
        "load\tpower_mean_w\tpower_p95_w\tenergy_j\tout_tok\tj_per_out\tn_power"
    )
    for st in sorted(stages, key=lambda x: float(x.get("load", 0))):
        t0 = int(st["t_start_ns"])
        t1 = int(st["t_end_ns"])
        load = float(st.get("load", st.get("stage", 0)))
        power_samples: list[tuple[int, float]] = []
        energy_samples: list[tuple[int, float]] = []
        for t in tele:
            tn = int(t["t_ns"])
            if tn < t0 or tn > t1:
                continue
            m = t.get("metric", "")
            if m in POWER_METRICS:
                power_samples.append((tn, float(t["value"])))
            if m in ENERGY_METRICS:
                energy_samples.append((tn, float(t["value"])))
        powers = [p for _, p in power_samples]
        energy = 0.0
        if len(energy_samples) >= 2:
            energy_samples.sort()
            energy = energy_samples[-1][1] - energy_samples[0][1]
        else:
            energy = trapezoid_energy(power_samples)
        out_tok = sum(
            int(r.get("output_tokens") or 0)
            for r in reqs
            if float(r.get("stage", -1)) == load and r.get("success")
        )
        jpt = energy / out_tok if out_tok else float("nan")
        print(
            f"{load:g}\t{mean(powers):.3f}\t{percentile(powers, 95):.3f}\t"
            f"{energy:.3f}\t{out_tok}\t{jpt:.6g}\t{len(powers)}"
        )

    summary = next((r for r in rows if r.get("kind") == "summary"), None)
    if summary:
        print("\n## summary")
        print(
            f"partial={summary.get('partial')} dropped_telemetry_rows={summary.get('dropped_telemetry_rows')}"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
