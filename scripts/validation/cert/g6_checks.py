#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Independent checks for vLLM usage, counters and candidate throughput."""
import base64
import json
import math

import metrics_oracle


def server_usage(trace_text):
    grouped = {}
    for line in trace_text.splitlines():
        event = json.loads(line)
        request = event["request"]
        record = grouped.setdefault(request, {"chunks": [], "complete": False})
        if event["kind"] == "chunk":
            record["chunks"].append(base64.b64decode(event["bytes_b64"], validate=True))
        elif event["kind"] == "complete":
            record["complete"] = True
        elif event["kind"] == "error":
            raise ValueError("proxy recorded upstream failure")
    result = []
    for record in grouped.values():
        if not record["complete"]:
            raise ValueError("incomplete recorded server response")
        data = b"".join(record["chunks"]).replace(b"\r\n", b"\n")
        usage = None
        reasoning_seen = False
        for frame in data.split(b"\n\n"):
            lines = [line[5:].lstrip() for line in frame.split(b"\n") if line.startswith(b"data:")]
            if not lines:
                continue
            content = b"\n".join(lines)
            if content == b"[DONE]":
                continue
            value = json.loads(content)
            for choice in value.get("choices") or []:
                delta = choice.get("delta") or {}
                if delta.get("reasoning_content") or delta.get("reasoning"):
                    reasoning_seen = True
            if value.get("usage") is not None:
                found = value["usage"]
                if usage is not None and usage != found:
                    raise ValueError("multiple inconsistent usage frames")
                usage = found
        if not isinstance(usage, dict):
            raise ValueError("server usage missing")
        for field in ("prompt_tokens", "completion_tokens"):
            if not isinstance(usage.get(field), int) or isinstance(usage[field], bool) or usage[field] < 0:
                raise ValueError(f"invalid server {field}")
        result.append({**usage, "reasoning_seen": reasoning_seen})
    return result


def evaluate(data_text, trace_text, before, after, model):
    rows = [json.loads(line) for line in data_text.splitlines() if line.strip()]
    requests = [row for row in rows if "request.v" in row.get("schema_version", "") and row.get("phase", "measure") == "measure"]
    summaries = [row for row in rows if "summary.v" in row.get("schema_version", "")]
    if len(requests) != 64 or not summaries:
        raise ValueError("64 measured requests and a summary are required")
    if any(row.get("error") is not None or row.get("usage_prompt_tokens_present") is not True or
           row.get("usage_completion_tokens_present") is not True for row in requests):
        raise ValueError("all measured requests must complete with full server usage")
    server = server_usage(trace_text)
    if len(server) != 64:
        raise ValueError("64 complete server traces with usage are required")
    reasoning_requests = sum(row["reasoning_seen"] for row in server)
    if reasoning_requests == 0:
        raise ValueError("no parser reasoning observed")
    input_tokens = sum(row["prompt_tokens"] for row in requests)
    output_tokens = sum(row["completion_tokens"] for row in requests)
    server_input = sum(row["prompt_tokens"] for row in server)
    server_output = sum(row["completion_tokens"] for row in server)
    if input_tokens != server_input or output_tokens != server_output:
        raise ValueError("CLI token sums do not match independently captured server usage")
    summary = summaries[-1]
    if summary.get("attempted") != 64 or summary.get("successes") != 64:
        raise ValueError("summary count mismatch")
    if summary.get("prompt_tokens_total") != input_tokens or summary.get("completion_tokens_total") != output_tokens:
        raise ValueError("summary usage totals disagree with request rows")
    window = summary.get("window_seconds")
    counters = metrics_oracle.compare(before, after, model, input_tokens, output_tokens, window)
    if counters["result"] != "PASS":
        raise ValueError("server token counters differ beyond tolerance")
    expected = counters["usage_rates"]
    actual = {"input_tokens_per_second": summary.get("input_tokens_per_second"),
              "output_tokens_per_second": summary.get("completion_tokens_per_second"),
              "total_tokens_per_second": summary.get("total_tokens_per_second")}
    for name, value in actual.items():
        if not isinstance(value, (int, float)) or isinstance(value, bool) or not math.isfinite(value) or not math.isclose(value, expected[name], rel_tol=0.01, abs_tol=1e-9):
            raise ValueError(f"rate mismatch: {name}")
    return {"requests": 64, "reasoning_requests": reasoning_requests, "prompt_tokens": input_tokens, "completion_tokens": output_tokens,
            "server_prompt_tokens": server_input, "server_completion_tokens": server_output,
            "window_seconds": window, "rates": actual, "counter_comparison": counters["counters"]}
