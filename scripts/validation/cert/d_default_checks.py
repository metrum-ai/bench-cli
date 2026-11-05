#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Assertions for the default-cap reasoning experience; no cap is injected."""
import json
import math

import r_checks


def evaluate(data, trace, stderr, exit_code):
    observed = r_checks.truncation_cell(data, trace, 64, False, exit_code)
    rows = [json.loads(line) for line in data.splitlines() if line.strip()]
    summary = next(row for row in rows if ".summary." in row.get("schema_version", ""))
    if not isinstance(summary.get("config"), dict) or not summary["config"]:
        raise ValueError("effective run configuration is missing")
    ttft = summary.get("ttft_s", {}).get("p50")
    rate = summary.get("completion_tokens_per_second")
    if not isinstance(ttft, (int, float)) or not math.isfinite(ttft) or ttft <= 0:
        raise ValueError("headline TTFT missing or non-positive")
    if not isinstance(rate, (int, float)) or not math.isfinite(rate) or rate <= 0:
        raise ValueError("completion throughput missing or non-positive")
    warning = f"warning: {observed['truncated']}/64 measured requests exhausted the token budget on reasoning without an answer"
    if warning not in stderr:
        raise ValueError("truncation warning with count and denominator missing")
    return {**observed, "headline_ttft_p50_s": ttft, "completion_tokens_per_second": rate,
            "truncation_fraction": observed["truncated"] / 64, "warning_present": True,
            "effective_config": summary.get("config")}
