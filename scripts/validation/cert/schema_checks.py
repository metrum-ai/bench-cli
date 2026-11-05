#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Validate live schema exports and version-aware migration invariants."""
import csv
import io
import json
import math

import g6_checks


def json_lines(text):
    return [json.loads(line) for line in text.splitlines() if line.strip()]


def equal_optional(a, b):
    if a is None or b is None:
        return a is None and b is None
    if isinstance(a, bool) or isinstance(b, bool):
        return a is b
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        return math.isclose(a, b, rel_tol=0, abs_tol=1e-9)
    return a == b


def evaluate(llm_text, llm_telemetry_text, llm_trace_text, strategic_text, strategic_csv_text, strategic_ndjson_text):
    llm = json_lines(llm_text)
    requests = [row for row in llm if row.get("schema_version") == "metrum-ai-bench-cli.request.v5" and row.get("phase") == "measure"]
    summaries = [row for row in llm if row.get("schema_version") == "metrum-ai-bench-cli.summary.v5"]
    if not requests or not summaries:
        raise ValueError("new request.v5 and summary.v5 output required")
    if any(row.get("schema_version", "").endswith((".summary.v3", ".summary.v4")) for row in llm):
        raise ValueError("old summary reader must reject this new-only output")
    if not any(row.get("answer_completed") is True for row in requests):
        raise ValueError("no visible answer in live request output")
    trace = g6_checks.server_usage(llm_trace_text)
    if not any(row["reasoning_seen"] for row in trace):
        raise ValueError("no actual parser reasoning in server stream")
    telemetry = json_lines(llm_telemetry_text)
    if not telemetry or telemetry[0].get("schema_version") != "metrum-ai-bench-cli.telemetry.v3":
        raise ValueError("LLM telemetry schema is not v3")
    telemetry_requests = {row["seq"]: row for row in telemetry if row.get("kind") == "request"}
    if len(telemetry_requests) != len(requests):
        raise ValueError("LLM telemetry/request count mismatch")
    for row in requests:
        other = telemetry_requests.get(row["seq"])
        if other is None:
            raise ValueError("LLM telemetry request missing")
        for field in ("ttft_s", "ttft_visible_answer_s", "ttft_source", "outcome", "answer_completed"):
            if not equal_optional(row.get(field), other.get(field)):
                raise ValueError(f"LLM telemetry field mismatch: {field}")
    strategic = json.loads(strategic_text)
    if strategic.get("schema_version") != "metrum-ai-bench-cli.strategic.v3" or not strategic.get("points"):
        raise ValueError("strategic.v3 summary and points required")
    strategic_rows = list(csv.DictReader(io.StringIO(strategic_csv_text)))
    if not strategic_rows:
        raise ValueError("strategic CSV has no request rows")
    required = {"seq", "ttft_s", "ttft_source", "ttft_visible_answer_s", "outcome", "answer_completed"}
    if not required <= set(strategic_rows[0]):
        raise ValueError("strategic CSV migration fields missing")
    strategic_ndjson = json_lines(strategic_ndjson_text)
    if not strategic_ndjson or strategic_ndjson[0].get("schema_version") != "metrum-ai-bench-cli.telemetry.v3":
        raise ValueError("strategic telemetry schema is not v3")
    strategic_requests = {str(row["seq"]): row for row in strategic_ndjson if row.get("kind") == "request"}
    if len(strategic_requests) != len(strategic_rows):
        raise ValueError("strategic telemetry/CSV count mismatch")
    for row in strategic_rows:
        other = strategic_requests.get(row["seq"])
        if other is None:
            raise ValueError("strategic telemetry request missing")
        for field in ("ttft_source", "outcome", "answer_completed"):
            expected = row[field].lower() if field == "answer_completed" else row[field]
            value = other.get(field)
            actual = str(value).lower() if isinstance(value, bool) else (value or "")
            if expected != actual:
                raise ValueError(f"strategic telemetry field mismatch: {field}")
        for field in ("ttft_s", "ttft_visible_answer_s"):
            expected = float(row[field]) if row[field] else None
            if not equal_optional(expected, other.get(field)):
                raise ValueError(f"strategic telemetry field mismatch: {field}")
    return {"llm_requests": len(requests), "llm_answered": sum(row.get("answer_completed") is True for row in requests),
            "reasoning_streams": sum(row["reasoning_seen"] for row in trace),
            "llm_telemetry_requests": len(telemetry_requests), "strategic_csv_rows": len(strategic_rows),
            "strategic_telemetry_requests": len(strategic_requests), "old_summary_reader_rejects_v5": True}
