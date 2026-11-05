#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Independent legacy-population comparison for the new 8B A/B grid.

This does not alter the existing 27B gates. Missing or incomparable evidence is
FAIL; it cannot establish that the inputs/noise floors were frozen in advance.
"""
import argparse
import datetime as dt
import json
import math
import re
import statistics
import sys
from pathlib import Path

import g6_checks

POINTS = tuple((w, c) for w in ("W1", "W2") for c in (1, 8, 32))
MATCHING_META = ("model", "model_revision", "image", "workload", "prompt_sha256", "concurrency",
                 "max_tokens", "seed", "temperature", "thinking", "warmup_seconds", "measure_seconds")


def _number(value, *, positive=False):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or (positive and value <= 0):
        raise ValueError("missing or invalid finite measurement")
    return float(value)


def _send_times(rows):
    # Never mix origins inside one run. Old request schemas fall back to RFC3339.
    if all(row.get("send_offset_s") is not None for row in rows):
        return [_number(row["send_offset_s"]) for row in rows]
    if any(row.get("send_offset_s") is not None for row in rows):
        raise ValueError("mixed timestamp origins")
    try:
        return [dt.datetime.fromisoformat(row["started_at"].replace("Z", "+00:00")).timestamp() for row in rows]
    except (KeyError, ValueError, TypeError) as exc:
        raise ValueError("missing comparable send clocks") from exc


def point(meta, data, trace, binary_sha):
    if not re.fullmatch(r"[0-9a-f]{64}", binary_sha) or meta.get("binary_sha256") != binary_sha or meta.get("thinking") is not False or meta.get("temperature") != 0 or meta.get("seed") != 42 or meta.get("warmup_seconds") != 30 or meta.get("measure_seconds") != 180:
        raise ValueError("run settings or executable pin differ from frozen grid")
    if meta.get("model") != "Qwen/Qwen3-8B" or not re.fullmatch(r"[0-9a-f]{40}", str(meta.get("model_revision", ""))) or not re.search(r"@sha256:[0-9a-f]{64}$", str(meta.get("image", ""))) or not re.fullmatch(r"[0-9a-f]{64}", str(meta.get("prompt_sha256", ""))):
        raise ValueError("model, server, revision or prompt pin missing")
    if meta.get("max_tokens") != {"W1": 128, "W2": 512}.get(meta.get("workload")):
        raise ValueError("workload output cap does not match frozen grid")
    rows = [json.loads(line) for line in data.splitlines() if line.strip()]
    summaries = [row for row in rows if ".summary." in row.get("schema_version", "")]
    if len(summaries) != 1 or summaries[0].get("partial", False):
        raise ValueError("exactly one complete summary is required")
    requests = [row for row in rows if ".request." in row.get("schema_version", "") and row.get("phase") == "measure"]
    if not requests or summaries[0].get("attempted") != len(requests):
        raise ValueError("missing or mismatched measured samples")
    # Mixed failure populations cannot be joined to the independent usage trace
    # without request IDs; block rather than compare incompatible numerators.
    if any(row.get("error") is not None or row.get("outcome") == "reasoning_truncated" for row in requests):
        raise ValueError("failure/truncation populations require an independent join")
    server = g6_checks.server_usage(trace)
    if len(server) != len(requests):
        raise ValueError("server usage coverage incomplete")
    output = 0
    tpot = []
    sends = _send_times(requests)
    issuance_span = max(sends) - min(sends)
    if issuance_span < 180.0:
        raise ValueError("real send timestamps do not prove a full 180-second issuance window")
    trace_rows = [json.loads(line) for line in trace.splitlines() if line.strip()]
    proxy_sends = [_number(event.get('monotonic_ns')) for event in trace_rows if event.get('kind') == 'send']
    if len(proxy_sends) != len(requests) or (max(proxy_sends) - min(proxy_sends)) / 1e9 < 180.0:
        raise ValueError("independent proxy timestamps do not prove a full 180-second issuance window")
    ends = []
    for row, send in zip(requests, sends):
        tokens = row.get("completion_tokens")
        if not isinstance(tokens, int) or isinstance(tokens, bool) or tokens < 2:
            raise ValueError("positive per-request output tokens required")
        if row.get("usage_completion_tokens_present") is False:
            raise ValueError("completion usage incomplete")
        first = _number(row.get("ttft_s"), positive=True)
        latency = _number(row.get("latency_s"), positive=True)
        if latency <= first:
            raise ValueError("invalid decode interval")
        if row.get("ttft_visible_answer_s") is not None and not math.isclose(first, _number(row["ttft_visible_answer_s"]), rel_tol=0, abs_tol=1e-9):
            raise ValueError("thinking-off first-any and visible clocks differ")
        output += tokens
        ends.append(send + latency)
        tpot.append((latency - first) / (tokens - 1))
    if output != sum(row["completion_tokens"] for row in server):
        raise ValueError("CLI output token sum does not match independent server usage")
    window = max(ends) - min(sends)
    if not math.isfinite(window) or window <= 0:
        raise ValueError("invalid legacy success window")
    return {"sample_count": len(requests), "output_tokens": output, "window_seconds": window, "issuance_span_seconds": issuance_span,
            "legacy_output_tokens_per_second": output / window, "tpot_p50_ms": statistics.median(tpot) * 1000}


def compare(base_root, candidate_root, base_sha, candidate_sha, noise):
    result = []
    for workload, concurrency in POINTS:
        key = f"{workload}/c{concurrency}"
        row = {"point": key, "result": "FAIL"}
        try:
            a = base_root / workload / f"c{concurrency}"
            b = candidate_root / workload / f"c{concurrency}"
            ma = json.loads((a / "point.json").read_text(encoding="utf-8"))
            mb = json.loads((b / "point.json").read_text(encoding="utf-8"))
            if any(ma.get(field) is None or ma.get(field) != mb.get(field) for field in MATCHING_META):
                raise ValueError("baseline and candidate settings, model, image or dataset pins differ")
            if ma["workload"] != workload or ma["concurrency"] != concurrency:
                raise ValueError("point identity does not match grid")
            x = point(ma, (a / "requests.jsonl").read_text(encoding="utf-8"), (a / "trace.jsonl").read_text(encoding="utf-8"), base_sha)
            y = point(mb, (b / "requests.jsonl").read_text(encoding="utf-8"), (b / "trace.jsonl").read_text(encoding="utf-8"), candidate_sha)
            checks = {}
            for metric in ("legacy_output_tokens_per_second", "tpot_p50_ms"):
                floor = _number(noise.get(key, {}).get(metric, 0))
                if floor < 0:
                    raise ValueError("negative noise floor")
                limit = max(5.0, floor)
                delta = 100 * (y[metric] - x[metric]) / x[metric]
                checks[metric] = {"delta_pct": delta, "tolerance_pct": limit, "pass": abs(delta) <= limit}
            row.update({"baseline": x, "candidate": y, "checks": checks,
                        "result": "PASS" if all(check["pass"] for check in checks.values()) else "FAIL"})
        except (OSError, ValueError, TypeError, KeyError, json.JSONDecodeError, ZeroDivisionError) as exc:
            row["error"] = str(exc)
        result.append(row)
    return {"result": "PASS" if all(row["result"] == "PASS" for row in result) else "FAIL", "points": result}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline-root", type=Path, required=True)
    parser.add_argument("--candidate-root", type=Path, required=True)
    parser.add_argument("--baseline-sha256", required=True)
    parser.add_argument("--candidate-sha256", required=True)
    parser.add_argument("--noise-floor", type=Path, help="previously frozen per-point noise-floor JSON")
    args = parser.parse_args()
    try:
        noise = json.loads(args.noise_floor.read_text(encoding="utf-8")) if args.noise_floor else {}
        if not isinstance(noise, dict):
            raise ValueError("noise floor must be an object")
        result = compare(args.baseline_root, args.candidate_root, args.baseline_sha256, args.candidate_sha256, noise)
        print(json.dumps(result, indent=2, allow_nan=False))
        return 0 if result["result"] == "PASS" else 1
    except (OSError, ValueError, TypeError, json.JSONDecodeError) as exc:
        print(f"comparison failed: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
