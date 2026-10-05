#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Stdlib-only analysis recipes for strategic telemetry NDJSON.

Usage:
  python3 docs/queries/analyze.py RUN.ndjson [STRATEGIC_STDOUT.json] [--json]

Prints row counts, then per measure stage: power (time-weighted mean, type 7
p95), energy from counter delta and from the power trapezoid, J/token, and the
docs/telemetry/ANALYSIS.md derived metrics (gpu_util_mean, sm_active_p50,
sm_occupancy_p50, tensor_active_p50, hollow_util_mean, kv_cache_util_mean,
preemptions_delta). Engine `_seconds` histograms get p50/p95 from bucket
deltas; a rank in the first bucket or in +Inf is reported as a bound
(`<=0.3`, `>60`), never interpolated from 0. Pass the strategic stdout JSON
to resolve the knee and report kv_cache_util_at_knee. --json prints the same result as one JSON object.

Utilization-style outputs are ratios in [0, 1]; percent gauges are scaled.
"""

from __future__ import annotations

import json
import math
import re
import sys
from collections import defaultdict
from typing import Any, Dict, List, Optional, Tuple


# One GPU power source and one GPU energy source per stage, in preference
# order (all-smi first, DCGM fallback, like the derived-metric families below).
# Summing two exporters that report the same GPU doubles power and energy.
# Node and chassis meters (ipmi_*, redfish_*, *chassis*) are wall power, not
# GPU power, and are never mixed in. Other names are a fallback only when none
# of these is present (see pick_metric).
POWER_PREFERENCE = [
    "all_smi_gpu_power_consumption_watts",
    "DCGM_FI_DEV_POWER_USAGE",
    "nvidia_smi_power_draw_watts",
    "nv_gpu_power_usage",
    "gpu_power_usage",
    "gpu_package_power",
    "hw_power",
]

ENERGY_PREFERENCE = [
    "all_smi_gpu_energy_hw_millijoules_total",
    "DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION",
    "nvidia_smi_energy_joules_total",
    "nv_energy_consumption",
    "gpu_energy_consumed",
    "hw_energy",
    "habanalabs_energy",
]

# Derived-metric sources, in preference order: the first metric with samples in
# the stage window wins. Value is the scale to a [0, 1] ratio. all-smi names
# match the recorded Metrum fork page (scripts/parity/fixtures/
# all-smi-fork-h100.prom, v0.26.3-metrum.4); DCGM PROF fields are the fallback
# when only dcgm-exporter is scraped.
GPU_UTIL = [
    ("all_smi_gpu_utilization", 0.01),  # percent
    ("DCGM_FI_DEV_GPU_UTIL", 0.01),  # percent
    ("nvidia_smi_utilization_gpu_ratio", 1.0),
    ("gpu_gfx_activity", 0.01),  # AMD, percent
]
SM_ACTIVE = [
    ("all_smi_gpu_sm_active_ratio", 1.0),  # DCGM field 1002 via GPM
    ("DCGM_FI_PROF_SM_ACTIVE", 1.0),
]
SM_OCCUPANCY = [
    ("all_smi_gpu_sm_occupancy", 1.0),  # GPM, Hopper and later
    ("DCGM_FI_PROF_SM_OCCUPANCY", 1.0),
]
TENSOR_ACTIVE = [
    ("all_smi_gpu_tensor_active_ratio", 1.0),  # DCGM field 1004 via GPM
    ("DCGM_FI_PROF_PIPE_TENSOR_ACTIVE", 1.0),
]
HOLLOW_UTIL = [("all_smi_gpu_hollow_utilization_ratio", 1.0)]
# DCGM fallback for hollow utilization, same definition as the fork gauge:
# graphics_active - sm_active, clamped at 0, paired per scrape and GPU.
HOLLOW_DCGM = ("DCGM_FI_PROF_GR_ENGINE_ACTIVE", "DCGM_FI_PROF_SM_ACTIVE")
# vLLM *_perc gauges are fractions (1.0 = full) despite the name.
KV_CACHE = [
    ("vllm:kv_cache_usage_perc", 1.0),
    ("vllm:gpu_cache_usage_perc", 1.0),  # pre-v1 vLLM name
    ("sglang:token_usage", 1.0),
    ("trtllm_kv_cache_utilization", 1.0),
    ("llamacpp:kv_cache_usage_ratio", 1.0),
]
PREEMPTIONS = ["vllm:num_preemptions_total", "sglang:num_preemptions_total"]

# Matches #190: Kneedle needs at least this many sweep points.
KNEE_MIN_POINTS = 5


# Power-named gauges that are configuration, not draw, or that meter the whole
# node. The name fallback below must skip them: all-smi exports
# power_limit_{current,max}_watts next to power_consumption_watts, and summing
# them reported ~992 W on a 350 W H100.
NOT_POWER_DRAW = re.compile(
    r"limit|cap|max|min|threshold|default|enforced|chassis|node|ipmi|redfish",
    re.IGNORECASE,
)


def is_power_draw(row: Dict[str, Any]) -> bool:
    metric = row.get("metric", "")
    if metric in POWER_PREFERENCE:
        return True
    return (
        "power" in metric.lower()
        and row.get("unit") == "W"
        and not NOT_POWER_DRAW.search(metric)
    )


def is_energy_counter(row: Dict[str, Any]) -> bool:
    metric = row.get("metric", "")
    if metric in ENERGY_PREFERENCE:
        return True
    return (
        "energy" in metric.lower()
        and row.get("unit") == "J"
        and not NOT_POWER_DRAW.search(metric)
    )


def pick_metric(window: List[Dict[str, Any]], preferred: List[str], matches) -> Optional[str]:
    """First `preferred` metric with rows in the window, else the
    lexicographically first other metric that `matches` (deterministic)."""
    present = {r.get("metric", "") for r in window if matches(r)}
    for metric in preferred:
        if metric in present:
            return metric
    return min(present) if present else None


def energy_joules(row: Dict[str, Any]) -> float:
    """Counter value in joules. Millijoule counters (all-smi
    all_smi_gpu_energy_hw_millijoules_total) are scaled by 0.001 unless the
    ingest already scaled them via a YAML `units:` entry (then `raw` is set)."""
    value = float(row["value"])
    if "millijoule" in row.get("metric", "").lower() and "raw" not in row:
        return value * 0.001
    return value


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
    """last - first. None with fewer than 2 samples or on a reset (any drop),
    which ANALYSIS.md treats as an invalid stage for that series."""
    if len(samples) < 2:
        return None
    samples = sorted(samples)
    if any(b[1] < a[1] for a, b in zip(samples, samples[1:])):
        return None
    return samples[-1][1] - samples[0][1]


def percentile_type7(values: List[float], p: float) -> Optional[float]:
    """Hyndman-Fan type 7 (numpy default, src/stats.rs percentile_type7)."""
    if not values:
        return None
    xs = sorted(values)
    n = len(xs)
    h = (n - 1) * (p / 100.0)
    lo = int(math.floor(h))
    hi = int(math.ceil(h))
    if lo == hi:
        return xs[lo]
    return xs[lo] * (hi - h) + xs[hi] * (h - lo)


BELOW_FIRST_BUCKET = "below_first_bucket"
ABOVE_LAST_BUCKET = "above_last_bucket"


def histogram_quantile(buckets: List[Tuple[float, float]], q: float) -> Dict[str, Any]:
    """Quantile on cumulative (le, count) buckets as {value, reason, bound}.

    Linear interpolation inside the bucket that holds rank q * total, as
    Prometheus histogram_quantile does, but only between two finite bounds.
    The first bucket has no lower bound, so a rank there is not interpolated
    from 0: value is None, reason is below_first_bucket and bound is the first
    finite `le` (the quantile is <= bound). A rank in +Inf is likewise None
    with reason above_last_bucket and bound the highest finite `le` (the
    quantile is > bound). All three are None when there are no observations
    or no finite bucket.
    """
    out: Dict[str, Any] = {"value": None, "reason": None, "bound": None}
    if not buckets:
        return out
    bs = sorted(buckets)
    total = bs[-1][1]
    if total <= 0:
        return out
    rank = q * total
    prev_le: Optional[float] = None
    prev_count = 0.0
    for le, count in bs:
        if count >= rank:
            if math.isinf(le):
                if prev_le is not None:
                    out.update(reason=ABOVE_LAST_BUCKET, bound=prev_le)
            elif prev_le is None:
                out.update(reason=BELOW_FIRST_BUCKET, bound=le)
            elif count == prev_count:
                out["value"] = le
            else:
                out["value"] = prev_le + (le - prev_le) * (rank - prev_count) / (count - prev_count)
            return out
        prev_le, prev_count = le, count
    return out


def fmt_quantile(hist: Dict[str, Any], name: str) -> str:
    """Text form of engine_histograms[m][name]: number, <=bound or >bound."""
    reason = hist.get(name + "_reason")
    if reason == BELOW_FIRST_BUCKET:
        return f"<={fmt(hist[name + '_bound'])}"
    if reason == ABOVE_LAST_BUCKET:
        return f">{fmt(hist[name + '_bound'])}"
    return fmt(hist[name])


def stage_histogram_quantile(
    window: List[Dict[str, Any]], metric: str, q: float
) -> Dict[str, Any]:
    """Quantile of `metric` (base name, no _bucket) observed inside a stage.

    Buckets are counters: take the delta per series (last - first in the
    window), sum deltas across non-`le` labels (engines, models), then apply
    histogram_quantile.
    """
    by_series: Dict[Any, List[Tuple[float, float]]] = defaultdict(list)
    for r in window:
        if r.get("metric") != metric + "_bucket":
            continue
        by_series[series_key(r)].append((r["t_ns"] / 1e9, float(r["value"])))
    # Group buckets by label set without `le`. A reset in any bucket drops the
    # whole label set: a partial histogram would skew the quantile.
    by_set: Dict[Any, Dict[float, float]] = defaultdict(dict)
    bad: set = set()
    for key, samples in by_series.items():
        labels = dict(key[2])
        le = labels.pop("le", None)
        if le is None:
            continue
        label_set = (key[0], tuple(sorted(labels.items())))
        d = counter_delta(samples)
        if d is None:
            bad.add(label_set)
            continue
        by_set[label_set][float(le)] = d
    by_le: Dict[float, float] = defaultdict(float)
    for label_set, buckets in by_set.items():
        if label_set in bad:
            continue
        for le, d in buckets.items():
            by_le[le] += d
    return histogram_quantile(list(by_le.items()), q)


def pick_family(
    window: List[Dict[str, Any]], family: List[Tuple[str, float]]
) -> Tuple[Optional[str], Dict[Any, List[Tuple[float, float]]]]:
    """First metric in `family` with samples in the window, as scaled series."""
    for metric, scale in family:
        series: Dict[Any, List[Tuple[float, float]]] = defaultdict(list)
        for r in window:
            if r.get("metric") == metric:
                series[series_key(r)].append((r["t_ns"] / 1e9, float(r["value"]) * scale))
        if series:
            return metric, series
    return None, {}


def mean_over_series(series: Dict[Any, List[Tuple[float, float]]]) -> Optional[float]:
    """Time-weighted mean per series (GPU), then the plain mean across them."""
    means = [m for m in (time_weighted_mean(s) for s in series.values()) if m is not None]
    return sum(means) / len(means) if means else None


def pooled_p50(series: Dict[Any, List[Tuple[float, float]]]) -> Optional[float]:
    """Type 7 median of all samples, pooled across GPUs."""
    return percentile_type7([v for s in series.values() for _, v in s], 50.0)


def hollow_from_dcgm(window: List[Dict[str, Any]]) -> Dict[Any, List[Tuple[float, float]]]:
    gr_metric, sm_metric = HOLLOW_DCGM
    by_gpu: Dict[Any, Dict[int, Dict[str, float]]] = defaultdict(lambda: defaultdict(dict))
    for r in window:
        metric = r.get("metric")
        if metric in HOLLOW_DCGM:
            gpu = (r.get("src", ""), series_key(r)[2])
            by_gpu[gpu][r["t_ns"]][metric] = float(r["value"])
    series: Dict[Any, List[Tuple[float, float]]] = defaultdict(list)
    for gpu, by_t in by_gpu.items():
        for t_ns, vals in by_t.items():
            if gr_metric in vals and sm_metric in vals:
                series[gpu].append((t_ns / 1e9, max(0.0, vals[gr_metric] - vals[sm_metric])))
    return series


def derived_metrics(window: List[Dict[str, Any]]) -> Dict[str, Any]:
    """ANALYSIS.md derived metrics for one stage window. `sources` names the
    metric each value came from (None when no candidate was scraped)."""
    out: Dict[str, Any] = {}
    sources: Dict[str, Optional[str]] = {}

    src, series = pick_family(window, GPU_UTIL)
    out["gpu_util_mean"], sources["gpu_util_mean"] = mean_over_series(series), src
    for name, family in (
        ("sm_active_p50", SM_ACTIVE),
        ("sm_occupancy_p50", SM_OCCUPANCY),
        ("tensor_active_p50", TENSOR_ACTIVE),
    ):
        src, series = pick_family(window, family)
        out[name], sources[name] = pooled_p50(series), src

    src, series = pick_family(window, HOLLOW_UTIL)
    if not series:
        series = hollow_from_dcgm(window)
        src = f"{HOLLOW_DCGM[0]} - {HOLLOW_DCGM[1]}" if series else None
    out["hollow_util_mean"], sources["hollow_util_mean"] = mean_over_series(series), src

    src, series = pick_family(window, KV_CACHE)
    out["kv_cache_util_mean"], sources["kv_cache_util_mean"] = mean_over_series(series), src

    preempt: Dict[Any, List[Tuple[float, float]]] = defaultdict(list)
    preempt_src = None
    for metric in PREEMPTIONS:
        for r in window:
            if r.get("metric") == metric:
                preempt[series_key(r)].append((r["t_ns"] / 1e9, float(r["value"])))
        if preempt:
            preempt_src = metric
            break
    deltas = [counter_delta(s) for s in preempt.values()]
    out["preemptions_delta"] = (
        sum(deltas) if deltas and all(d is not None for d in deltas) else None  # type: ignore[misc]
    )
    sources["preemptions_delta"] = preempt_src

    hist: Dict[str, Dict[str, Any]] = {}
    for base in sorted(
        {r["metric"][: -len("_bucket")] for r in window
         if r.get("metric", "").endswith("_seconds_bucket")}
    ):
        hist[base] = {}
        for name, q in (("p50", 0.50), ("p95", 0.95)):
            qv = stage_histogram_quantile(window, base, q)
            hist[base][name] = qv["value"]
            hist[base][name + "_reason"] = qv["reason"]
            hist[base][name + "_bound"] = qv["bound"]
    out["engine_histograms"] = hist
    out["sources"] = sources
    return out


def resolve_knee(stdout: Optional[Dict[str, Any]]) -> Dict[str, Any]:
    """Knee load from strategic stdout JSON.

    Prefers `knee_detection` (#190). Older outputs lack it: fall back to the
    `knee` field, but only for sweeps of at least KNEE_MIN_POINTS points,
    since #190 marks 3- and 4-point knees as unreliable.
    """
    if stdout is None:
        return {"load": None, "index": None, "reason": "no_strategic_stdout",
                "source": None, "note": "pass the strategic stdout JSON to resolve the knee"}
    points = stdout.get("points") or []
    det = stdout.get("knee_detection")
    if isinstance(det, dict):
        index = det.get("index")
        if index is None:
            return {"load": None, "index": None, "reason": det.get("reason"),
                    "source": "knee_detection",
                    "note": f"no knee: {det.get('reason')} "
                            f"({det.get('points')} points, min {det.get('min_points')})"}
        if not isinstance(index, int) or not 0 <= index < len(points):
            return {"load": None, "index": None, "reason": "knee_index_out_of_range",
                    "source": "knee_detection",
                    "note": f"knee_detection.index {index!r} is outside points[0..{len(points)})"}
        return {"load": points[index].get("load"), "index": index, "reason": None,
                "source": "knee_detection", "note": None}
    legacy = "knee_detection absent (output predates #190); used the legacy knee field"
    # Like #190, count only points with a p95 latency.
    usable = sum(1 for p in points if p.get("p95_s") is not None)
    if usable < KNEE_MIN_POINTS:
        return {"load": None, "index": None, "reason": "insufficient_points",
                "source": "legacy_knee",
                "note": f"{legacy}; {usable} points with p95_s < {KNEE_MIN_POINTS}, "
                        "so any legacy knee is ignored"}
    knee = stdout.get("knee")
    if knee is None:
        return {"load": None, "index": None, "reason": "legacy_knee_null",
                "source": "legacy_knee", "note": f"{legacy}; it is null and gives no reason"}
    index = next((i for i, p in enumerate(points) if p.get("load") == knee.get("load")), None)
    return {"load": knee.get("load"), "index": index, "reason": None,
            "source": "legacy_knee", "note": legacy}


def analyze(rows: List[Dict[str, Any]], stdout: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
    by_kind: Dict[str, int] = defaultdict(int)
    for r in rows:
        by_kind[r.get("kind", "?")] += 1

    stages = [r for r in rows if r.get("kind") == "stage" and r.get("phase") == "measure"]
    tele = [r for r in rows if r.get("kind") == "telemetry"]
    reqs = [r for r in rows if r.get("kind") == "request" and not r.get("warmup")]

    out_stages: List[Dict[str, Any]] = []
    for s in sorted(stages, key=lambda x: (x.get("stage", 0), x.get("load", 0))):
        t0 = s["t_start_ns"]
        t1 = s["t_end_ns"]
        window = [
            r for r in tele if r.get("run_id") == s.get("run_id") and t0 <= r["t_ns"] < t1
        ]

        # One power source; sum its GPUs at the same t_ns (board total), then
        # time-weight.
        power_metric = pick_metric(window, POWER_PREFERENCE, is_power_draw)
        power_by_t: Dict[int, float] = defaultdict(float)
        power_values: List[float] = []
        for r in window:
            if r.get("metric") == power_metric:
                power_by_t[r["t_ns"]] += float(r["value"])
                power_values.append(float(r["value"]))
        power_series = [(t / 1e9, v) for t, v in sorted(power_by_t.items())]
        power_mean = time_weighted_mean(power_series)
        power_p95 = percentile_type7([v for _, v in power_series] or power_values, 95.0)
        energy_trap = trapezoid_energy_j(power_series)

        # One energy counter source: Δ per series, then sum series (multi-GPU).
        energy_metric = pick_metric(window, ENERGY_PREFERENCE, is_energy_counter)
        energy_by_series: Dict[Any, List[Tuple[float, float]]] = defaultdict(list)
        for r in window:
            if r.get("metric") == energy_metric:
                energy_by_series[series_key(r)].append((r["t_ns"] / 1e9, energy_joules(r)))
        energy_counter = 0.0
        energy_counter_ok = False
        for samples in energy_by_series.values():
            d = counter_delta(samples)
            if d is not None:
                energy_counter += d
                energy_counter_ok = True

        stage_reqs = [
            r for r in reqs
            if r.get("run_id") == s.get("run_id")
            and r.get("stage") == s.get("stage")
            and r.get("success")
        ]
        out_tokens = sum(int(r.get("output_tokens") or 0) for r in stage_reqs)
        energy_for_jtok = energy_counter if energy_counter_ok else energy_trap
        j_per = (
            energy_for_jtok / out_tokens
            if energy_for_jtok is not None and out_tokens > 0
            else None
        )
        derived = derived_metrics(window)
        derived["sources"] = {
            "power": power_metric, "energy_counter": energy_metric, **derived["sources"]
        }
        out_stages.append({
            "run_id": s.get("run_id"),
            "stage": s.get("stage"),
            "load": s.get("load"),
            "power_n": len(power_series),
            "power_mean_w": power_mean,
            "power_p95_w": power_p95,
            "energy_counter_j": energy_counter if energy_counter_ok else None,
            "energy_trap_j": energy_trap,
            "out_tokens": out_tokens,
            "j_per_output_token": j_per,
            **derived,
        })

    knee = resolve_knee(stdout)
    at_knee: Optional[float] = None
    reason = knee["reason"]
    if knee["load"] is not None:
        match = [s for s in out_stages if s["load"] == knee["load"]]
        if not match:
            reason = "knee_stage_not_in_ndjson"
        else:
            at_knee = match[0]["kv_cache_util_mean"]
            if at_knee is None:
                reason = (
                    "no_kv_cache_series" if match[0]["sources"]["kv_cache_util_mean"] is None
                    else "insufficient_kv_samples"
                )
    return {
        "row_counts": dict(sorted(by_kind.items())),
        "stages": out_stages,
        "knee": knee,
        "kv_cache_util_at_knee": at_knee,
        "kv_cache_util_at_knee_reason": reason,
    }


def fmt(x: Any) -> str:
    if x is None or (isinstance(x, float) and not math.isfinite(x)):
        return ""
    if isinstance(x, float):
        return f"{x:.6g}"
    return str(x)


def render(result: Dict[str, Any]) -> None:
    print("## row counts")
    for kind, n in result["row_counts"].items():
        print(f"{kind}\t{n}")

    print("\n## stage power / energy")
    cols = ["stage", "load", "power_n", "power_mean_w", "power_p95_w",
            "energy_counter_j", "energy_trap_j", "out_tokens", "j_per_output_token"]
    print("\t".join(cols))
    for s in result["stages"]:
        print("\t".join(fmt(s[c]) for c in cols))

    print("\n## stage utilization / KV (ratios 0-1)")
    cols = ["stage", "load", "gpu_util_mean", "sm_active_p50", "sm_occupancy_p50",
            "tensor_active_p50", "hollow_util_mean", "kv_cache_util_mean",
            "preemptions_delta"]
    print("\t".join(cols))
    sources: Dict[str, set] = defaultdict(set)
    for s in result["stages"]:
        print("\t".join(fmt(s[c]) for c in cols))
        for k, v in s["sources"].items():
            sources[k].add(v or "absent")
    print("\n## sources")
    for k in ["power", "energy_counter"] + cols[2:]:
        print(f"{k}\t{', '.join(sorted(sources[k])) or 'absent'}")

    hist_rows = [(s, m, q) for s in result["stages"] for m, q in s["engine_histograms"].items()]
    if hist_rows:
        print("\n## engine histograms (bucket deltas, seconds)")
        print("stage\tload\tmetric\tp50\tp95")
        for s, m, q in hist_rows:
            print(f"{fmt(s['stage'])}\t{fmt(s['load'])}\t{m}\t"
                  f"{fmt_quantile(q, 'p50')}\t{fmt_quantile(q, 'p95')}")

    knee = result["knee"]
    print("\n## knee")
    print(f"knee_load\t{fmt(knee['load'])}")
    print(f"knee_source\t{knee['source'] or ''}")
    at = result["kv_cache_util_at_knee"]
    print(f"kv_cache_util_at_knee\t{'null' if at is None else fmt(at)}")
    if result["kv_cache_util_at_knee_reason"]:
        print(f"kv_cache_util_at_knee_reason\t{result['kv_cache_util_at_knee_reason']}")
    if knee["note"]:
        print(f"note\t{knee['note']}")


def main(argv: List[str]) -> int:
    args = [a for a in argv[1:] if a != "--json"]
    if not 1 <= len(args) <= 2:
        print("usage: analyze.py RUN.ndjson [STRATEGIC_STDOUT.json] [--json]", file=sys.stderr)
        return 2
    stdout = None
    if len(args) == 2:
        with open(args[1], encoding="utf-8") as f:
            stdout = json.load(f)
    result = analyze(load_rows(args[0]), stdout)
    if "--json" in argv[1:]:
        print(json.dumps(result, indent=2, sort_keys=True))
    else:
        render(result)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
