#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Assertions for the real reasoning-parser release cases."""
import json
import math

import g6_checks


def load_cell(data, trace, count):
    rows = [json.loads(line) for line in data.splitlines() if line.strip()]
    requests = [row for row in rows if "request.v" in row.get("schema_version", "") and row.get("phase", "measure") == "measure"]
    summaries = [row for row in rows if "summary.v" in row.get("schema_version", "")]
    if len(requests) != count or not summaries or summaries[-1].get("attempted") != count:
        raise ValueError(f"expected {count} measured rows and matching summary")
    if any(row.get("usage_prompt_tokens_present") is not True or row.get("usage_completion_tokens_present") is not True for row in requests):
        raise ValueError("server usage coverage incomplete")
    server = g6_checks.server_usage(trace)
    if len(server) != count:
        raise ValueError("real server response/usage coverage incomplete")
    if sum(row["prompt_tokens"] for row in requests) != sum(row["prompt_tokens"] for row in server) or \
       sum(row["completion_tokens"] for row in requests) != sum(row["completion_tokens"] for row in server):
        raise ValueError("CLI usage differs from independently recorded server usage")
    return requests, summaries[-1], server


def truncation_cell(data, trace, count, strict, exit_code):
    requests, summary, server = load_cell(data, trace, count)
    truncated = [row for row in requests if row.get("outcome") == "reasoning_truncated"]
    if not truncated or not any(row["reasoning_seen"] for row in server):
        raise ValueError("required parser reasoning and no-answer truncation not observed")
    if any(row.get("error") is not None and (not strict or row.get("outcome") != "reasoning_truncated") for row in requests):
        raise ValueError("unexpected non-truncation request error")
    for row in truncated:
        if row.get("answer_completed") is not False or row.get("ttft_s") is None:
            raise ValueError("truncation did not preserve first-any time and unanswered outcome")
        if strict and (row.get("error") or {}).get("kind") != "reasoning_truncated":
            raise ValueError("strict truncation did not become an error")
        if not strict and row.get("error") is not None:
            raise ValueError("default truncation was classified as an error")
    if strict and exit_code == 0:
        raise ValueError("strict mode must exit nonzero on truncation")
    if not strict and exit_code != 0:
        raise ValueError("default truncation run failed")
    if summary.get("ttft_coverage", {}).get("reasoning_truncated") != len(truncated):
        raise ValueError("truncation coverage does not match rows")
    return {"attempted": len(requests), "truncated": len(truncated), "exit_code": exit_code,
            "reasoning_streams": sum(row["reasoning_seen"] for row in server)}


def adequate_cell(data, trace, count, exit_code):
    requests, summary, server = load_cell(data, trace, count)
    answered = [row for row in requests if row.get("answer_completed") is True]
    if not answered or not any(row["reasoning_seen"] for row in server):
        raise ValueError("need a real reasoning stream and at least one answer")
    for row in answered:
        first, visible = row.get("ttft_s"), row.get("ttft_visible_answer_s")
        if not isinstance(first, (int, float)) or not isinstance(visible, (int, float)) or first > visible:
            raise ValueError("first-any and visible-answer clocks are inconsistent")
    if exit_code != 0 or any(row.get("error") is not None for row in requests):
        raise ValueError("adequate-budget candidate has errors or exited nonzero")
    total, window, rate = summary.get("completion_tokens_total"), summary.get("window_seconds"), summary.get("completion_tokens_per_second")
    if total != sum(row["completion_tokens"] for row in requests):
        raise ValueError("completion numerator disagrees with records")
    if not isinstance(window, (int, float)) or window <= 0 or not isinstance(rate, (int, float)) or not math.isclose(rate, total / window, rel_tol=1e-9):
        raise ValueError("completion throughput does not match recorded numerator and window")
    return {"attempted": count, "answered": len(answered), "reasoning_streams": sum(row["reasoning_seen"] for row in server),
            "completion_tokens": summary.get("completion_tokens_total"), "window_seconds": summary.get("window_seconds"), "exit_code": exit_code}


def safety_cell(data, trace, count, exit_code):
    requests, summary, server = load_cell(data, trace, count)
    errors = sum(row.get("error") is not None for row in requests)
    if (errors > 0 and exit_code == 0) or (errors == 0 and exit_code != 0):
        raise ValueError("CI safety exit does not reflect measured errors")
    if summary.get("errors") != errors or not any(row["reasoning_seen"] for row in server):
        raise ValueError("CI safety error count or reasoning evidence missing")
    for row in requests:
        if row.get("outcome") == "reasoning_truncated" and (row.get("error") or {}).get("kind") != "reasoning_truncated":
            raise ValueError("CI safety did not enforce strict truncation")
    return {"attempted": count, "errors": errors, "exit_code": exit_code}


def control_cell(data, trace, count, exit_code):
    requests, _, _ = load_cell(data, trace, count)
    if exit_code != 0 or any(row.get("answer_completed") is not True or row.get("error") is not None or
                             row.get("outcome") == "reasoning_truncated" for row in requests):
        raise ValueError("thinking-off control did not answer every request")
    for row in requests:
        first, visible = row.get("ttft_s"), row.get("ttft_visible_answer_s")
        if not isinstance(first, (float, int)) or not isinstance(visible, (float, int)) or not math.isclose(first, visible, rel_tol=0, abs_tol=1e-9):
            raise ValueError("control first-any and visible clocks differ")
    return {"attempted": count, "answered": count, "exit_code": exit_code}


def thinking_live_cell(data, trace, count, exit_code):
    requests, _, server = load_cell(data, trace, count)
    if exit_code != 0 or any(row.get("error") is not None or row.get("ttft_s") is None for row in requests):
        raise ValueError("thinking-on run did not complete without errors and clocks")
    reasoning = sum(row["reasoning_seen"] for row in server)
    if not reasoning:
        raise ValueError("no parser-emitted reasoning observed")
    return {"attempted": count, "reasoning_streams": reasoning, "exit_code": exit_code}
