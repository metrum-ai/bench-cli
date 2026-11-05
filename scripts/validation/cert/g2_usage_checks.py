#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Verify real-stream usage omissions against retained original and forwarded bytes."""
import base64
import json

import g1_clock_checks


def parse_sse(raw):
    frames = raw.replace(b"\r\n", b"\n").split(b"\n\n")
    result = []
    for frame in frames:
        data = b"\n".join(line[5:].lstrip() for line in frame.splitlines() if line.startswith(b"data:"))
        if data and data != b"[DONE]":
            result.append(json.loads(data))
    return result


def evaluate(data, trace, code, mode, count):
    if mode not in ("control", "remove_usage", "remove_reasoning_split"):
        raise ValueError("unknown usage test mode")
    clock = g1_clock_checks.evaluate(data, trace, code)
    records = [json.loads(line) for line in data.splitlines() if line.strip()]
    requests = [row for row in records if ".request." in row.get("schema_version", "") and row.get("phase") == "measure"]
    summaries = [row for row in records if ".summary." in row.get("schema_version", "")]
    if len(requests) != count or len(summaries) != 1:
        raise ValueError("usage request or summary coverage incomplete")
    summary = summaries[0]
    groups = {}
    for row in (json.loads(line) for line in trace.splitlines() if line.strip()):
        groups.setdefault(row["request"], []).append(row)
    if len(groups) != count:
        raise ValueError("real stream evidence count mismatch")
    for events in groups.values():
        original = b"".join(base64.b64decode(e["bytes_b64"], validate=True) for e in events if e["kind"] == "chunk")
        forwarded = b"".join(base64.b64decode(e["bytes_b64"], validate=True) for e in events if e["kind"] == "forwarded_chunk")
        origin_usage = [frame["usage"] for frame in parse_sse(original) if isinstance(frame.get("usage"), dict)]
        forwarded_usage = [frame["usage"] for frame in parse_sse(forwarded) if isinstance(frame.get("usage"), dict)]
        if len(origin_usage) != 1:
            raise ValueError("real server did not supply exactly one usage block")
        transforms = [e for e in events if e["kind"] == "transformation"]
        if mode == "control":
            if original != forwarded or len(forwarded_usage) != 1 or transforms:
                raise ValueError("control stream changed")
        elif mode == "remove_usage":
            if forwarded_usage or not transforms or any(e.get("mode") != mode for e in transforms):
                raise ValueError("usage block not removed from recorded server stream")
        else:
            before = origin_usage[0].get("completion_tokens_details")
            if not isinstance(before, dict) or "reasoning_tokens" not in before or len(forwarded_usage) != 1:
                raise ValueError("server did not supply reasoning split")
            after = forwarded_usage[0].get("completion_tokens_details")
            if not isinstance(after, dict) or "reasoning_tokens" in after or not transforms or any(e.get("mode") != mode for e in transforms):
                raise ValueError("reasoning split not removed")
            expected = dict(origin_usage[0]); expected_details = dict(before); del expected_details["reasoning_tokens"]
            expected["completion_tokens_details"] = expected_details
            if forwarded_usage[0] != expected:
                raise ValueError("other usage fields changed")
    if mode == "control":
        if any(row.get("usage_prompt_tokens_present") is not True or row.get("usage_completion_tokens_present") is not True for row in requests):
            raise ValueError("control usage not reflected in records")
    elif mode == "remove_usage":
        if any(row.get("usage_prompt_tokens_present") is not False or row.get("usage_completion_tokens_present") is not False for row in requests):
            raise ValueError("missing usage coverage flags incorrect")
        if any(summary.get(field) is not None for field in ("input_tokens_per_second", "completion_tokens_per_second", "total_tokens_per_second")):
            raise ValueError("incomplete usage rates must be null")
    else:
        if any(row.get("reasoning_tokens") is not None or row.get("usage_completion_tokens_present") is not True for row in requests):
            raise ValueError("missing split or total usage coverage flags incorrect")
        if summary.get("non_reasoning_completion_tokens_per_second") is not None:
            raise ValueError("incomplete split rate must be null")
    return {"requests": count, "mode": mode, "clock_check": clock}
