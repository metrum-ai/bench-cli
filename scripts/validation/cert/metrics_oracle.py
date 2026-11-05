#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Compare isolated vLLM Prometheus token counters with complete usage totals.

This oracle does not start a server or establish that snapshots are isolated;
the operator must capture the evidence and verify the matching population.
"""
import argparse
import json
import math
import re
import sys
from pathlib import Path

SAMPLE = re.compile(r'^([a-zA-Z_:][a-zA-Z0-9_:]*)(\{[^}]*\})?\s+([^\s]+)(?:\s+[^\s]+)?\s*$')
LABEL = re.compile(r'\s*([a-zA-Z_][a-zA-Z0-9_]*)\s*=\s*("(?:[^"\\]|\\.)*")\s*(?:,|$)')
METRICS = ("vllm:prompt_tokens_total", "vllm:generation_tokens_total")


def parse_labels(value):
    if not value:
        return ()
    value = value[1:-1]
    labels = {}
    offset = 0
    while offset < len(value):
        match = LABEL.match(value, offset)
        if match is None or match.group(1) in labels:
            raise ValueError("malformed or duplicate Prometheus label")
        labels[match.group(1)] = json.loads(match.group(2))
        offset = match.end()
    return tuple(sorted(labels.items()))


def snapshot(source, model):
    result = {}
    for raw in source.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        match = SAMPLE.fullmatch(line)
        if not match:
            if any(line.startswith(name) for name in METRICS):
                raise ValueError("malformed token counter")
            continue
        name = match.group(1)
        if name not in METRICS:
            continue
        labels = parse_labels(match.group(2))
        if dict(labels).get("model_name") != model:
            continue
        try:
            value = float(match.group(3))
        except ValueError as exc:
            raise ValueError("invalid counter value") from exc
        if not math.isfinite(value) or value < 0:
            raise ValueError("invalid counter value")
        key = (name, labels)
        if key in result:
            raise ValueError("duplicate counter series")
        result[key] = value
    if any(not any(key[0] == name for key in result) for name in METRICS):
        raise ValueError("required model token counter missing")
    return result


def compare(before_text, after_text, model, prompt_tokens, completion_tokens, window_seconds):
    before = snapshot(before_text, model)
    after = snapshot(after_text, model)
    if before.keys() != after.keys():
        raise ValueError("counter series changed between snapshots")
    if any(after[key] < before[key] for key in before):
        raise ValueError("counter reset detected")
    for value in (prompt_tokens, completion_tokens):
        if not isinstance(value, int) or isinstance(value, bool) or value < 0:
            raise ValueError("usage totals must be non-negative integers")
    if not isinstance(window_seconds, (float, int)) or isinstance(window_seconds, bool) or not math.isfinite(window_seconds) or window_seconds <= 0:
        raise ValueError("window must be positive and finite")
    totals = {name: sum(after[key] - before[key] for key in before if key[0] == name) for name in METRICS}
    expected = {METRICS[0]: prompt_tokens, METRICS[1]: completion_tokens}
    results = {}
    for name in METRICS:
        difference = abs(totals[name] - expected[name])
        tolerance = max(1.0, 0.05 * expected[name])
        results[name] = {"counter_delta": totals[name], "usage_total": expected[name],
                         "absolute_difference": difference, "tolerance": tolerance,
                         "pass": difference <= tolerance}
    return {"result": "PASS" if all(item["pass"] for item in results.values()) else "FAIL",
            "counters": results, "window_seconds": window_seconds,
            "usage_rates": {"input_tokens_per_second": prompt_tokens / window_seconds,
                            "output_tokens_per_second": completion_tokens / window_seconds,
                            "total_tokens_per_second": (prompt_tokens + completion_tokens) / window_seconds}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, required=True)
    parser.add_argument("--model", required=True)
    parser.add_argument("--prompt-tokens", type=int, required=True)
    parser.add_argument("--completion-tokens", type=int, required=True)
    parser.add_argument("--window-seconds", type=float, required=True)
    args = parser.parse_args()
    try:
        result = compare(args.before.read_text(encoding="utf-8"), args.after.read_text(encoding="utf-8"),
                         args.model, args.prompt_tokens, args.completion_tokens, args.window_seconds)
    except (OSError, ValueError) as exc:
        print(f"metrics oracle failed: {exc}", file=sys.stderr)
        return 2
    print(json.dumps(result, sort_keys=True, allow_nan=False))
    return 0 if result["result"] == "PASS" else 1


if __name__ == "__main__":
    sys.exit(main())
