#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Join serial real SSE streams to CLI clocks with explicit uncertainty."""
import base64
import json
import math


def stream_clocks(trace):
    groups = {}
    for event in (json.loads(line) for line in trace.splitlines() if line.strip()):
        groups.setdefault(event["request"], []).append(event)
    output = []
    for events in groups.values():
        send = next((e["monotonic_ns"] for e in events if e["kind"] == "send"), None)
        finish = next((e["monotonic_ns"] for e in events if e["kind"] == "complete"), None)
        if send is None or finish is None or any(e["kind"] == "error" for e in events):
            raise ValueError("incomplete real proxy stream")
        buf = b""
        first = visible = None
        reasoning_seen = False
        for event in events:
            if event["kind"] != "chunk":
                continue
            buf += base64.b64decode(event["bytes_b64"], validate=True).replace(b"\r\n", b"\n")
            while b"\n\n" in buf:
                frame, buf = buf.split(b"\n\n", 1)
                data = b"\n".join(line[5:].lstrip() for line in frame.splitlines() if line.startswith(b"data:"))
                if not data or data == b"[DONE]":
                    continue
                item = json.loads(data)
                for choice in item.get("choices", []):
                    if choice.get("index", 0) != 0:
                        continue
                    delta = choice.get("delta") or {}
                    reason = delta.get("reasoning_content") or delta.get("reasoning")
                    answer = delta.get("content")
                    tools = delta.get("tool_calls") or delta.get("function_call")
                    if reason:
                        reasoning_seen = True
                    if first is None and (reason or answer or tools):
                        first = event["monotonic_ns"]
                    if visible is None and answer:
                        visible = event["monotonic_ns"]
        if buf.strip():
            raise ValueError("unterminated SSE frame")
        output.append({"send": send, "finish": finish, "first": first, "visible": visible,
                       "reasoning_seen": reasoning_seen})
    return sorted(output, key=lambda row: row["send"])


def evaluate(data, trace, exit_code):
    rows = [json.loads(line) for line in data.splitlines() if line.strip()]
    requests = sorted((row for row in rows if ".request." in row.get("schema_version", "") and row.get("phase") == "measure"), key=lambda row: row["seq"])
    streams = stream_clocks(trace)
    if exit_code != 0 or not requests or len(requests) != len(streams):
        raise ValueError("serial request and real stream coverage incomplete")
    reasoning = compared = visible_compared = 0
    for row, stream in zip(requests, streams):
        if row.get("error") is not None or stream["first"] is None:
            raise ValueError("request error or missing real first token")
        latency = row.get("latency_s")
        if not isinstance(latency, (float, int)) or not math.isfinite(latency):
            raise ValueError("invalid client latency")
        proxy_duration = (stream["finish"] - stream["send"]) / 1e9
        uncertainty = abs(latency - proxy_duration)
        if uncertainty > 0.05:
            raise ValueError("client/proxy timing uncertainty exceeds 50ms")
        first_proxy = (stream["first"] - stream["send"]) / 1e9
        first_cli = row.get("ttft_s")
        if not isinstance(first_cli, (float, int)) or abs(first_cli - first_proxy) > 0.05 or row.get("ttft_source") != "stream":
            raise ValueError("first-any timing or streaming provenance mismatch")
        compared += 1
        if stream["visible"] is not None:
            visible = row.get("ttft_visible_answer_s")
            visible_proxy = (stream["visible"] - stream["send"]) / 1e9
            if not isinstance(visible, (float, int)) or abs(visible - visible_proxy) > 0.05:
                raise ValueError("visible-answer timing mismatch")
            visible_compared += 1
        elif row.get("ttft_visible_answer_s") is not None:
            raise ValueError("CLI visible-answer clock without server answer")
        reasoning += int(stream["reasoning_seen"])
    if not reasoning or not visible_compared:
        raise ValueError("reasoning and visible-answer streams required")
    return {"requests": len(requests), "reasoning_streams": reasoning, "first_any_compared": compared,
            "visible_compared": visible_compared, "max_clock_error_s": 0.05}
