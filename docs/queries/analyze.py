#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Stdlib-only analysis recipes for strategic telemetry NDJSON.

Usage:
  python3 docs/queries/analyze.py /path/to/run.ndjson

Prints row counts, per-measure-stage power (time-weighted mean), energy from
counter delta when present, trapezoid energy from power gauges, and J/token.
"""

from __future__ import annotations

import json
import math
import sys
from collections import defaultdict
from typing import Any, Dict, List, Optional, Tuple


POWER_METRICS = {
    "DCGM_FI_DEV_POWER_USAGE",
    "all_smi_gpu_power_consumption_watts",
    "nvidia_smi_power_draw_watts",
    "gpu_power_usage",
    "gpu_package_power",
    "hw_power",
    "ipmi_dcmi_power_consumption_current_watts",
    "ipmi_power_watts",
    "nv_gpu_power_usage",
    "redfish_chassis_power_average_consumed_watts",
}

ENERGY_METRICS = {
    "DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION",
    "nvidia_smi_energy_joules_total",
    "gpu_energy_consumed",
    "hw_energy",
    "habanalabs_energy",
    "nv_energy_consumption",
}


def load_rows(path: str) -> List[Dict[str, Any]]:
    rows: List[Dict[str, Any]] = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            rows.append(json.loads(line))
    return rows


def series_key(row: Dict[str, Any]) -> Tuple[str, str, Tuple[Tuple[str, str], ...]]:
    labels = row.get("labels") or {}
    return (
        row.get("src", ""),
        row.get("metric", ""),
        tuple(sorted((str(k), str(v)) for k, v in labels.items())),
    )


def time_weighted_mean(samples: List[Tuple[float, float]]) -> Optional[float]:
    if len(samples) < 2:
        return None
    samples = sorted(samples)
    num = 0.0
    den = 0.0
    for (t0, v0), (t1, v1) in zip(samples, samples[1:]):
        dt = t1 - t0
        if dt <= 0:
            continue
        num += 0.5 * (v0 + v1) * dt
        den += dt
    if den <= 0:
        return None
    return num / den


def trapezoid_energy_j(samples: List[Tuple[float, float]]) -> Optional[float]:
    """Integrate watts over seconds -> joules."""
    if len(samples) < 2:
        return None
    samples = sorted(samples)
    energy = 0.0
    for (t0, v0), (t1, v1) in zip(samples, samples[1:]):
        dt = t1 - t0
        if dt <= 0:
            continue
        energy += 0.5 * (v0 + v1) * dt
    return energy


def counter_delta(samples: List[Tuple[float, float]]) -> Optional[float]:
    if len(samples) < 2:
        return None
    samples = sorted(samples)
    return samples[-1][1] - samples[0][1]


def percentile_nearest(values: List[float], p: float) -> Optional[float]:
    if not values:
        return None
    xs = sorted(values)
    if len(xs) == 1:
        return xs[0]
    # Hyndman-Fan type 7
    n = len(xs)
    h = (n - 1) * (p / 100.0)
    lo = int(math.floor(h))
    hi = int(math.ceil(h))
    if lo == hi:
        return xs[lo]
    return xs[lo] * (hi - h) + xs[hi] * (h - lo)


def main(argv: List[str]) -> int:
    if len(argv) != 2:
        print("usage: analyze.py PATH.ndjson", file=sys.stderr)
        return 2
    path = argv[1]
    rows = load_rows(path)

    by_kind: Dict[str, int] = defaultdict(int)
    for r in rows:
        by_kind[r.get("kind", "?")] += 1
    print("## row counts")
    for kind in sorted(by_kind):
        print(f"{kind}\t{by_kind[kind]}")

    stages = [
        r
        for r in rows
        if r.get("kind") == "stage" and r.get("phase") == "measure"
    ]
    tele = [r for r in rows if r.get("kind") == "telemetry"]
    reqs = [r for r in rows if r.get("kind") == "request" and not r.get("warmup")]

    print("\n## stage power / energy")
    print(
        "stage\tload\tpower_n\tpower_mean_w\tpower_p95_w\t"
        "energy_counter_j\tenergy_trap_j\tout_tokens\tj_per_out_tok"
    )
    for s in sorted(stages, key=lambda x: (x.get("stage", 0), x.get("load", 0))):
        t0 = s["t_start_ns"]
        t1 = s["t_end_ns"]
        window = [
            r
            for r in tele
            if r.get("run_id") == s.get("run_id") and t0 <= r["t_ns"] < t1
        ]

        # Sum multi-GPU power at the same t_ns (board total), then time-weight.
        power_by_t: Dict[int, float] = defaultdict(float)
        power_values: List[float] = []
        for r in window:
            if r.get("metric") in POWER_METRICS or (
                "power" in r.get("metric", "").lower() and r.get("unit") == "W"
            ):
                power_by_t[r["t_ns"]] += float(r["value"])
                power_values.append(float(r["value"]))
        power_series = [(t / 1e9, v) for t, v in sorted(power_by_t.items())]
        power_mean = time_weighted_mean(power_series)
        power_p95 = percentile_nearest(
            [v for _, v in power_series] or power_values, 95.0
        )
        energy_trap = trapezoid_energy_j(power_series)

        # Energy counters: Δ per series, then sum series (multi-GPU).
        energy_by_series: Dict[Any, List[Tuple[float, float]]] = defaultdict(list)
        for r in window:
            metric = r.get("metric", "")
            if metric in ENERGY_METRICS or (
                r.get("unit") == "J" and "energy" in metric.lower()
            ):
                energy_by_series[series_key(r)].append(
                    (r["t_ns"] / 1e9, float(r["value"]))
                )
        energy_counter = 0.0
        energy_counter_ok = False
        for samples in energy_by_series.values():
            d = counter_delta(samples)
            if d is not None:
                energy_counter += d
                energy_counter_ok = True

        stage_reqs = [
            r
            for r in reqs
            if r.get("run_id") == s.get("run_id")
            and r.get("stage") == s.get("stage")
            and r.get("success")
        ]
        out_tokens = sum(int(r.get("output_tokens") or 0) for r in stage_reqs)
        energy_for_jtok = (
            energy_counter
            if energy_counter_ok
            else (energy_trap if energy_trap is not None else None)
        )
        j_per = (
            energy_for_jtok / out_tokens
            if energy_for_jtok is not None and out_tokens > 0
            else None
        )

        def fmt(x: Optional[float]) -> str:
            if x is None or (isinstance(x, float) and not math.isfinite(x)):
                return ""
            return f"{x:.6g}"

        print(
            f"{s.get('stage')}\t{s.get('load')}\t{len(power_series)}\t"
            f"{fmt(power_mean)}\t{fmt(power_p95)}\t"
            f"{fmt(energy_counter if energy_counter_ok else None)}\t"
            f"{fmt(energy_trap)}\t{out_tokens}\t{fmt(j_per)}"
        )

    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
