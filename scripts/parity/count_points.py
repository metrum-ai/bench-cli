#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Metrum AI Bench CLI parity harness: count the data points each tool reports.

Counts what a reader of each tool's output files actually gets. Built by
Metrum AI for issue #204; the rules are the ones that reproduce the epic #184
table (AIPerf 0.13.0 plain: 65 / 653 / 33).

  quantity     one reported measurement. AIPerf: each top-level block with a
               "unit" in profile_export_aiperf.json. Bench: every dict holding
               n, p50 and p99 (a DistSummary) at any depth, named by its dotted
               path (a DistSummary at summary["a"]["b"] is quantity "a.b"),
               each top-level numeric scalar, and each top-level block whose
               numeric leaves outside any DistSummary are non-empty (goodput,
               observed_concurrency, ...). Those leftover leaves, including
               leaves of nested non-distribution dicts, form the block.
  values       statistic slots inside those quantities. Inside a non-empty
               distribution (Bench DistSummary with n >= 1, AIPerf block with
               a "unit" and count >= 1 or no count) every statistic the block
               defines counts whether it is numeric or null, so std, null for
               n < 2, does not make the count depend on the sample size
               (#245). Those nulls are also reported as null_values. An empty
               distribution (n == 0 / count == 0) holds no data: only its
               numeric leaves count (its n), as before, and it is reported in
               empty_dists. Outside distributions only numeric leaves count.
               Booleans and strings never count.
  per-request  distinct numeric fields on measured per-request records.
               AIPerf: metric names under "metrics" on profiling-phase rows of
               profile_export.jsonl. Bench: non-null numeric leaves on
               phase=measure request.v3 rows, minus "seq" and "error". Nested
               dicts flatten to dotted names (modality_metrics.prompt_words);
               a list of numbers (itl_s) counts once.
  duplicates   quantities whose numbers equal an earlier quantity's numbers
               (for example ttft vs time_to_first_output_token), compared
               after rounding each number to 12 decimals. Quantities with a
               single number are never compared: lone scalars collide on 0 or
               1 by chance. Duplicates still count as quantities; the column
               shows how many are copies.
  dists        quantities with a full distribution (AIPerf 15 numbers, Bench
               DistSummary 10); "blocks" are AIPerf time-weighted blocks
               (8 numbers) or Bench non-distribution blocks.

Excluded on both sides: run metadata (Bench config/environment/sut, AIPerf
input_config/run_info), warmup data, Bench per_endpoint (a per-endpoint copy
of headline distributions; reported as per_endpoint_quantities), and
error-shaped data whose presence depends on the error rate: Bench
errors_by_type and per-request error.*, AIPerf error_summary and top-level
error_* blocks. Those are reported as error_quantities instead, so the
headline counts do not shift when a request fails.

Pacing guard: the run must carry a TTFT/E2E pair for every measured success
(streaming on, fields present, AIPerf profile_export.jsonl present). Zero
pairs, fewer pairs than successes, or a median TTFT/E2E above
--max-ttft-ratio (default 0.9; the mock returned the stream at once) all exit
3 with the reason. A missing input file is an error, not a skip.

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
BENCH_ERROR_KEYS = {"errors_by_type"}
AIPERF_EXCLUDE = {"input_config", "run_info", "warmup_metrics", "error_summary"}
DIST_KEYS_BENCH = {"n", "p50", "p99"}
REQUEST_SKIP = {"seq", "error"}


class PacingError(RuntimeError):
    """Raised when TTFT is about E2E: a single-chunk mock, not a stream."""


def is_num(v: Any) -> bool:
    return isinstance(v, (int, float)) and not isinstance(v, bool)


def is_slot(v: Any) -> bool:
    """A distribution statistic slot: numeric or null (an undefined statistic)."""
    return v is None or is_num(v)


def dist_slots(node: dict, size_key: str) -> list[float | None]:
    """The counted statistics of a distribution dict, in key order.

    Non-empty (node[size_key] >= 1, or no size_key): every statistic it
    defines, nulls kept. Empty (node[size_key] == 0): only its numeric leaves,
    so the undefined statistics of a distribution with no samples never count
    as data points."""
    if is_empty_dist(node, size_key):
        return [float(v) for v in node.values() if is_num(v)]
    return [None if v is None else float(v) for v in node.values() if is_slot(v)]


def is_empty_dist(node: dict, size_key: str) -> bool:
    """True when the distribution's sample size field is present and 0."""
    return is_num(node.get(size_key)) and node[size_key] == 0


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


def find_duplicates(quantities: dict[str, list[float | None]]) -> list[list[str]]:
    """Groups of quantity names whose number vectors are identical.

    Numbers are rounded to 12 decimals before comparing (a null slot matches
    only a null); quantities with a single slot are skipped (see the module
    docstring)."""
    groups: dict[tuple, list[str]] = {}
    for name, vals in quantities.items():
        if len(vals) < 2:
            continue  # lone scalars collide on 0/1 by chance; not a copy
        key = tuple(None if v is None else round(v, 12) for v in vals)
        groups.setdefault(key, []).append(name)
    return [names for names in groups.values() if len(names) > 1]


def null_count(quantities: dict[str, list[float | None]]) -> int:
    """Counted slots that are null (undefined for this run's sample size)."""
    return sum(1 for vals in quantities.values() for v in vals if v is None)


def pacing_check(pairs: Iterable[tuple[Any, Any]], successes: int, limit: float, tool: str) -> float:
    """Median TTFT/E2E over measured successes; PacingError when it cannot be trusted."""
    ratios = [t / e for t, e in pairs if is_num(t) and is_num(e) and e > 0]
    if successes == 0:
        raise PacingError(f"{tool}: no measured successful requests, nothing to count.")
    if len(ratios) < successes:
        raise PacingError(
            f"{tool}: only {len(ratios)} of {successes} measured successes carry a TTFT/E2E pair. "
            "Streaming must be on and every success needs ttft and e2e, otherwise the pacing "
            "guard cannot prove the stream was paced.")
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
    quantities: dict[str, list[float | None]] = {}
    empty: list[str] = []
    dists = blocks = 0
    for key, val in summary.items():
        if key in BENCH_EXCLUDE or key in BENCH_ERROR_KEYS:
            continue
        if is_num(val):
            quantities[key] = [float(val)]
        elif isinstance(val, dict):
            found: dict[str, list[float | None]] = {}
            leftover = split_dists(val, key, found, empty)
            for name, nums in found.items():
                quantities[name] = nums
                dists += 1
            if leftover:
                quantities[key] = leftover
                blocks += 1
    error_q = sum(1 for k in BENCH_ERROR_KEYS for v in (summary.get(k) or {}).values()
                  if numeric_leaves(v))
    per_ep = summary.get("per_endpoint") or {}
    per_ep_q = sum(1 for ep in per_ep.values() if isinstance(ep, dict)
                   for v in ep.values() if is_num(v) or (isinstance(v, dict) and numeric_leaves(v)))
    measured = [r for r in rows if str(r.get("schema_version", "")).endswith(".request.v3")
                and r.get("phase") == "measure"]
    fields: set[str] = set()
    for r in measured:
        fields |= request_fields(r)
    ok = [r for r in measured if not r.get("error")]
    med = pacing_check(((r.get("ttft_s"), r.get("latency_s")) for r in ok), len(ok), limit, "bench")
    dups = find_duplicates(quantities)
    return {
        "tool": "bench",
        "tool_version": (summary.get("environment") or {}).get("package_version"),
        "quantities": len(quantities),
        "values": sum(len(v) for v in quantities.values()),
        "null_values": null_count(quantities),
        "empty_dists": len(empty),
        "per_request": len(fields),
        "duplicates": sum(len(g) - 1 for g in dups),
        "dists": dists,
        "blocks": blocks,
        "measured_requests": len(measured),
        "per_endpoint_quantities": per_ep_q,
        "error_quantities": error_q,
        "median_ttft_over_e2e": med,
        "duplicate_groups": dups,
        "quantity_names": sorted(quantities),
        "empty_dist_names": sorted(empty),
        "per_request_fields": sorted(fields),
    }


def split_dists(node: dict, path: str, found: dict[str, list[float | None]],
                empty: list[str] | None = None) -> list[float]:
    """Record every DistSummary under node in found (by dotted path); return the
    numeric leaves that are not inside any DistSummary."""
    if DIST_KEYS_BENCH <= set(node):
        found[path] = dist_slots(node, "n")
        if empty is not None and is_empty_dist(node, "n"):
            empty.append(path)
        return []
    leftover: list[float] = []
    for k, v in node.items():
        if is_num(v):
            leftover.append(float(v))
        elif isinstance(v, dict):
            leftover.extend(split_dists(v, f"{path}.{k}", found, empty))
        elif isinstance(v, list):
            for i, item in enumerate(v):
                if isinstance(item, dict):
                    leftover.extend(split_dists(item, f"{path}.{k}[{i}]", found, empty))
                else:
                    leftover.extend(numeric_leaves(item))
    return leftover


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
    quantities: dict[str, list[float | None]] = {}
    empty = []
    dists = blocks = error_q = 0
    for key, val in summary.items():
        if key in AIPERF_EXCLUDE or not (isinstance(val, dict) and "unit" in val):
            continue
        if key.startswith("error_"):
            error_q += 1
            continue
        nums = dist_slots({k: v for k, v in val.items() if k != "unit"}, "count")
        if not any(v is not None for v in nums):
            continue  # nothing reported at all
        quantities[key] = nums
        if is_empty_dist(val, "count"):
            empty.append(key)
        if len(nums) == 1:
            continue
        if "count" in val and "p1" in val:
            dists += 1
        else:
            blocks += 1
    if not records_path.is_file():
        raise SystemExit(f"{art}: missing profile_export.jsonl (per-request records); "
                         "rerun AIPerf with the default --export-level")
    fields: set[str] = set()
    pairs = []
    n_meas = n_ok = 0
    for r in read_jsonl(records_path):
        if (r.get("metadata") or {}).get("benchmark_phase") != "profiling":
            continue
        n_meas += 1
        if r.get("error"):
            continue
        n_ok += 1
        metrics = r.get("metrics") or {}
        fields |= {k for k, v in metrics.items() if isinstance(v, dict) and v.get("value") is not None}
        pairs.append(((metrics.get("time_to_first_token") or {}).get("value"),
                      (metrics.get("request_latency") or {}).get("value")))
    med = pacing_check(pairs, n_ok, limit, "aiperf")
    dups = find_duplicates(quantities)
    return {
        "tool": "aiperf",
        "tool_version": summary.get("aiperf_version"),
        "quantities": len(quantities),
        "values": sum(len(v) for v in quantities.values()),
        "null_values": null_count(quantities),
        "empty_dists": len(empty),
        "per_request": len(fields),
        "duplicates": sum(len(g) - 1 for g in dups),
        "dists": dists,
        "blocks": blocks,
        "measured_requests": n_meas,
        "error_quantities": error_q,
        "median_ttft_over_e2e": med,
        "duplicate_groups": dups,
        "quantity_names": sorted(quantities),
        "empty_dist_names": sorted(empty),
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
        lines += ["", "| Scenario   | Tool   | dists | empty dists | blocks | duplicates | null values | measured | median TTFT/E2E |",
                  "|------------|--------|-------|-------------|--------|------------|-------------|----------|-----------------|"]
        for sc in client:
            for tool in ("aiperf", "bench"):
                c = by[sc].get(tool)
                if c:
                    med = c.get("median_ttft_over_e2e")
                    lines.append(f"| {sc:<10} | {tool:<6} | {c['dists']:>5} | "
                                 f"{c.get('empty_dists', 'n/a'):>11} | {c['blocks']:>6} | "
                                 f"{c['duplicates']:>10} | {c.get('null_values', 'n/a'):>11} | "
                                 f"{c['measured_requests']:>8} | "
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
                                   if k not in ("quantity_names", "per_request_fields",
                                                "empty_dist_names")}
    print(json.dumps(shown, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
