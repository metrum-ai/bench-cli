#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Metrum AI Bench CLI parity harness: count the data points each tool reports.

Counts what a reader of each tool's output files actually gets. Built by
Metrum AI for issue #204; the rules are the ones that reproduce the epic #184
table (AIPerf 0.13.0 plain: 65 / 653 / 33).

  quantity     one reported measurement. AIPerf: each top-level block with a
               "unit" in profile_export_aiperf.json. Bench: each DistSummary,
               each top-level numeric scalar, and each other nested block with
               numeric leaves (goodput, observed_concurrency, isl_osl, ...).
  values       numeric leaves inside those quantities (booleans and strings
               never count; null never counts).
  per-request  distinct numeric fields on measured per-request records.
               AIPerf: metric names under "metrics" on profiling-phase rows of
               profile_export.jsonl. Bench: non-null numeric leaves (a list
               counts once) on phase=measure request.v3 rows, minus "seq".
  duplicates   quantities whose numbers equal an earlier quantity's numbers
               exactly (for example ttft vs time_to_first_output_token). They
               still count as quantities; the column shows how many are copies.
  dists        quantities with a full distribution (AIPerf 15 numbers, Bench
               DistSummary 10); "blocks" are AIPerf time-weighted blocks
               (8 numbers) or Bench nested non-distribution blocks.

Excluded on both sides: run metadata (Bench config/environment/sut, AIPerf
input_config/run_info), warmup data, and Bench per_endpoint (a per-endpoint
copy of headline distributions; reported as per_endpoint_quantities).

Pacing guard: if the median of TTFT / E2E across measured requests is above
--max-ttft-ratio (default 0.9), the mock returned the stream at once and the
count is meaningless. The script exits 3 and prints why.

Subcommands:
  bench DATA_LOG.jsonl          count one Bench --data-log
  aiperf ARTIFACT_DIR           count one AIPerf --artifact-dir
  tele-bench RUN.ndjson         count Bench strategic telemetry rows
  tele-aiperf ARTIFACT_DIR      count AIPerf server_metrics_export.json
  table COUNTS.jsonl            print the scenario table (epic format)
Each count subcommand prints one JSON object; add --scenario NAME to label it
and --append FILE to also append it to a JSONL file for `table`.
Standard library only.
"""

from __future__ import annotations

import argparse
import json
import statistics
import sys
from pathlib import Path
from typing import Any, Iterable

BENCH_EXCLUDE = {"config", "environment", "sut", "per_endpoint", "schema_version"}
AIPERF_EXCLUDE = {"input_config", "run_info", "warmup_metrics", "error_summary"}
DIST_KEYS_BENCH = {"n", "p50", "p99"}
REQUEST_SKIP = {"seq"}


class PacingError(RuntimeError):
    """Raised when TTFT is about E2E: a single-chunk mock, not a stream."""


def is_num(v: Any) -> bool:
    return isinstance(v, (int, float)) and not isinstance(v, bool)


def numeric_leaves(node: Any) -> list[float]:
    """All numeric leaves under node, depth first, in key order."""
    if is_num(node):
        return [float(node)]
    if isinstance(node, dict):
        out: list[float] = []
        for v in node.values():
            out.extend(numeric_leaves(v))
        return out
    if isinstance(node, list):
        out = []
        for v in node:
            out.extend(numeric_leaves(v))
        return out
    return []


def find_duplicates(quantities: dict[str, list[float]]) -> list[list[str]]:
    """Groups of quantity names whose number vectors are identical (>= 2 numbers)."""
    groups: dict[tuple, list[str]] = {}
    for name, vals in quantities.items():
        if len(vals) < 2:
            continue  # lone scalars collide on 0/1 by chance; not a copy
        groups.setdefault(tuple(round(v, 12) for v in vals), []).append(name)
    return [names for names in groups.values() if len(names) > 1]


def pacing_check(pairs: Iterable[tuple[float, float]], limit: float, tool: str) -> float | None:
    ratios = [t / e for t, e in pairs if t is not None and e and e > 0]
    if not ratios:
        return None
    med = statistics.median(ratios)
    if med > limit:
        raise PacingError(
            f"{tool}: median TTFT/E2E = {med:.3f} > {limit}. The server returned the stream at "
            "once, so TTFT equals E2E and every streaming metric is meaningless. Use "
            "scripts/parity/mock_server.py (paced) and never count against a single-chunk mock.")
    return med


def read_jsonl(path: Path) -> list[dict]:
    rows = []
    with path.open(encoding="utf-8") as fh:
        for ln in fh:
            ln = ln.strip()
            if ln:
                rows.append(json.loads(ln))
    return rows


def count_bench(path: Path, limit: float) -> dict[str, Any]:
    rows = read_jsonl(path)
    summaries = [r for r in rows if str(r.get("schema_version", "")).endswith(".summary.v3")]
    if not summaries:
        raise SystemExit(f"{path}: no metrum-ai-bench-cli.summary.v3 line (run interrupted?)")
    summary = summaries[-1]
    quantities: dict[str, list[float]] = {}
    dists = blocks = 0
    for key, val in summary.items():
        if key in BENCH_EXCLUDE:
            continue
        if is_num(val):
            quantities[key] = [float(val)]
        elif isinstance(val, dict):
            nums = numeric_leaves(val)
            if not nums:
                continue
            quantities[key] = nums
            if DIST_KEYS_BENCH <= set(val):
                dists += 1
            else:
                blocks += 1
    per_ep = summary.get("per_endpoint") or {}
    per_ep_q = sum(1 for ep in per_ep.values() if isinstance(ep, dict)
                   for v in ep.values() if is_num(v) or (isinstance(v, dict) and numeric_leaves(v)))
    measured = [r for r in rows if str(r.get("schema_version", "")).endswith(".request.v3")
                and r.get("phase") == "measure"]
    fields: set[str] = set()
    for r in measured:
        fields |= request_fields(r)
    med = pacing_check(((r.get("ttft_s"), r.get("latency_s")) for r in measured if not r.get("error")),
                       limit, "bench")
    dups = find_duplicates(quantities)
    return {
        "tool": "bench",
        "tool_version": (summary.get("environment") or {}).get("package_version"),
        "quantities": len(quantities),
        "values": sum(len(v) for v in quantities.values()),
        "per_request": len(fields),
        "duplicates": sum(len(g) - 1 for g in dups),
        "dists": dists,
        "blocks": blocks,
        "measured_requests": len(measured),
        "per_endpoint_quantities": per_ep_q,
        "median_ttft_over_e2e": med,
        "duplicate_groups": dups,
        "quantity_names": sorted(quantities),
        "per_request_fields": sorted(fields),
    }


def request_fields(row: dict, prefix: str = "") -> set[str]:
    out: set[str] = set()
    for k, v in row.items():
        if not prefix and k in REQUEST_SKIP:
            continue
        name = f"{prefix}{k}"
        if is_num(v):
            out.add(name)
        elif isinstance(v, list) and v and all(is_num(x) for x in v):
            out.add(name)
        elif isinstance(v, dict):
            out |= request_fields(v, name + ".")
    return out


def count_aiperf(art: Path, limit: float) -> dict[str, Any]:
    summary_path = art / "profile_export_aiperf.json"
    records_path = art / "profile_export.jsonl"
    if not summary_path.is_file():
        raise SystemExit(f"{art}: missing profile_export_aiperf.json")
    summary = json.loads(summary_path.read_text(encoding="utf-8"))
    quantities: dict[str, list[float]] = {}
    dists = blocks = 0
    for key, val in summary.items():
        if key in AIPERF_EXCLUDE or not (isinstance(val, dict) and "unit" in val):
            continue
        nums = [float(x) for x in val.values() if is_num(x)]
        if not nums:
            continue
        quantities[key] = nums
        if len(nums) == 1:
            continue
        if "count" in val and "p1" in val:
            dists += 1
        else:
            blocks += 1
    fields: set[str] = set()
    pairs = []
    n_meas = 0
    if records_path.is_file():
        for r in read_jsonl(records_path):
            if (r.get("metadata") or {}).get("benchmark_phase") != "profiling":
                continue
            n_meas += 1
            metrics = r.get("metrics") or {}
            fields |= {k for k, v in metrics.items() if isinstance(v, dict) and v.get("value") is not None}
            ttft = (metrics.get("time_to_first_token") or {}).get("value")
            e2e = (metrics.get("request_latency") or {}).get("value")
            if ttft is not None and e2e:
                pairs.append((ttft, e2e))
    med = pacing_check(pairs, limit, "aiperf")
    dups = find_duplicates(quantities)
    return {
        "tool": "aiperf",
        "tool_version": summary.get("aiperf_version"),
        "quantities": len(quantities),
        "values": sum(len(v) for v in quantities.values()),
        "per_request": len(fields),
        "duplicates": sum(len(g) - 1 for g in dups),
        "dists": dists,
        "blocks": blocks,
        "measured_requests": n_meas,
        "median_ttft_over_e2e": med,
        "duplicate_groups": dups,
        "quantity_names": sorted(quantities),
        "per_request_fields": sorted(fields),
    }


def count_tele_bench(path: Path) -> dict[str, Any]:
    names: dict[str, set[str]] = {}
    series: set[tuple] = set()
    samples = 0
    for r in read_jsonl(path):
        if r.get("kind") != "telemetry":
            continue
        samples += 1
        src = r.get("src", "?")
        names.setdefault(src, set()).add(r["metric"])
        series.add((src, r["metric"], tuple(sorted((r.get("labels") or {}).items()))))
    return {
        "tool": "bench",
        "telemetry_values": samples,
        "telemetry_series": len(series),
        "names_by_source": {k: len(v) for k, v in sorted(names.items())},
        "all_smi_gpu_names": len({n for v in names.values() for n in v if n.startswith("all_smi_gpu_")}),
        "kind": "raw samples on the request clock",
    }


def count_tele_aiperf(art: Path) -> dict[str, Any]:
    path = art / "server_metrics_export.json"
    if not path.is_file():
        raise SystemExit(f"{art}: missing server_metrics_export.json")
    data = json.loads(path.read_text(encoding="utf-8"))
    metrics = data.get("metrics") or {}
    names_by_url: dict[str, set[str]] = {}
    values = n_series = 0
    for name, block in metrics.items():
        for s in block.get("series") or []:
            n_series += 1
            names_by_url.setdefault(s.get("endpoint_url", "?"), set()).add(name)
            values += len(numeric_leaves(s.get("stats"))) + len(numeric_leaves(s.get("buckets")))
    return {
        "tool": "aiperf",
        "telemetry_values": values,
        "telemetry_series": n_series,
        "names_by_source": {k: len(v) for k, v in sorted(names_by_url.items())},
        "all_smi_gpu_names": len({n for n in metrics if n.startswith("all_smi_gpu_")}),
        "kind": "aggregated stats per series (profiling phase)",
    }


def table(path: Path) -> str:
    rows = read_jsonl(path)
    by: dict[str, dict[str, dict]] = {}
    order: list[str] = []
    for r in rows:
        sc = r.get("scenario", "?")
        if sc not in by:
            by[sc] = {}
            order.append(sc)
        by[sc][r["tool"]] = r
    lines = []
    client = [s for s in order if not s.startswith("telemetry")]
    if client:
        lines += ["| Scenario   | AIPerf quantities / values / per-request | Bench quantities / values / per-request |",
                  "|------------|------------------------------------------|-----------------------------------------|"]
        for sc in client:
            cell = []
            for tool in ("aiperf", "bench"):
                c = by[sc].get(tool)
                cell.append(f"{c['quantities']} / {c['values']} / {c['per_request']}" if c else "n/a")
            lines.append(f"| {sc:<10} | {cell[0]:<40} | {cell[1]:<39} |")
        lines += ["", "| Scenario   | Tool   | dists | blocks | duplicates | measured | median TTFT/E2E |",
                  "|------------|--------|-------|--------|------------|----------|-----------------|"]
        for sc in client:
            for tool in ("aiperf", "bench"):
                c = by[sc].get(tool)
                if c:
                    med = c.get("median_ttft_over_e2e")
                    lines.append(f"| {sc:<10} | {tool:<6} | {c['dists']:>5} | {c['blocks']:>6} | "
                                 f"{c['duplicates']:>10} | {c['measured_requests']:>8} | "
                                 f"{(f'{med:.3f}' if med is not None else 'n/a'):>15} |")
    for sc in (s for s in order if s.startswith("telemetry")):
        lines += ["", f"Telemetry ({sc}):", "",
                  "| Tool   | values | series | all_smi_gpu names | names by source |",
                  "|--------|--------|--------|-------------------|-----------------|"]
        for tool in ("aiperf", "bench"):
            c = by[sc].get(tool)
            if c:
                src = ", ".join(f"{k}={v}" for k, v in c["names_by_source"].items())
                lines.append(f"| {tool:<6} | {c['telemetry_values']:>6} | {c['telemetry_series']:>6} | "
                             f"{c['all_smi_gpu_names']:>17} | {src} |")
    return "\n".join(lines)


def main() -> int:
    ap = argparse.ArgumentParser(description="Metrum AI parity data-point counter")
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name, arg in (("bench", "data_log"), ("aiperf", "artifact_dir"),
                      ("tele-bench", "ndjson"), ("tele-aiperf", "artifact_dir")):
        p = sub.add_parser(name)
        p.add_argument(arg, type=Path)
        p.add_argument("--scenario", default="")
        p.add_argument("--append", type=Path, help="also append the JSON object to this JSONL file")
        p.add_argument("--max-ttft-ratio", type=float, default=0.9)
        p.add_argument("--full", action="store_true", help="include name lists in stdout")
    t = sub.add_parser("table")
    t.add_argument("counts", type=Path)
    args = ap.parse_args()
    if args.cmd == "table":
        print(table(args.counts))
        return 0
    try:
        if args.cmd == "bench":
            out = count_bench(args.data_log, args.max_ttft_ratio)
        elif args.cmd == "aiperf":
            out = count_aiperf(args.artifact_dir, args.max_ttft_ratio)
        elif args.cmd == "tele-bench":
            out = count_tele_bench(args.ndjson)
        else:
            out = count_tele_aiperf(args.artifact_dir)
    except PacingError as err:
        print(f"count_points: {err}", file=sys.stderr)
        return 3
    out = {"scenario": args.scenario, **out}
    if args.append:
        with args.append.open("a", encoding="utf-8") as fh:
            fh.write(json.dumps(out) + "\n")
    shown = out if args.full else {k: v for k, v in out.items()
                                   if k not in ("quantity_names", "per_request_fields")}
    print(json.dumps(shown, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
