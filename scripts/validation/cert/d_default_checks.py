#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Assertions for the r3 field-reported reasoning configuration."""
import json
import math

import r_checks


def evaluate(data, trace, stderr, exit_code):
    observed = r_checks.truncation_cell(data, trace, 64, False, exit_code)
    rows = [json.loads(line) for line in data.splitlines() if line.strip()]
    summary = next(row for row in rows if ".summary." in row.get("schema_version", ""))
    config = summary.get("config")
    if not isinstance(config, dict) or not isinstance(config.get("body_template"), dict):
        raise ValueError("effective run configuration is missing")
    body = config["body_template"]
    if body.get("max_tokens") != 512 or body.get("chat_template_kwargs", {}).get("enable_thinking") is not True:
        raise ValueError("effective field-reported 512-token thinking configuration differs")
    ttft = summary.get("ttft_s", {}).get("p50")
    rate = summary.get("completion_tokens_per_second")
    if isinstance(ttft, bool) or not isinstance(ttft, (int, float)) or not math.isfinite(ttft):
        raise ValueError("headline TTFT missing or non-finite")
    if not isinstance(rate, (int, float)) or not math.isfinite(rate) or rate <= 0:
        raise ValueError("completion throughput missing or non-positive")
    warning = f"warning: {observed['truncated']}/64 measured requests exhausted the token budget on reasoning without an answer"
    if warning not in stderr:
        raise ValueError("truncation warning with count and denominator missing")
    return {**observed, "headline_ttft_p50_s": ttft, "completion_tokens_per_second": rate,
            "truncation_fraction": observed["truncated"] / 64, "warning_present": True,
            "effective_config": summary.get("config")}
