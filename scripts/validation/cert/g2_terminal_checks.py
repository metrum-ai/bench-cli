#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Check terminal faults applied exclusively to recorded real SSE frames."""
import base64
import json

import g2_usage_checks
import g6_checks
import r_checks


def fault_cell(data, trace, code, mode, count):
    if mode not in ("remove_output", "remove_finish_and_done", "nonlength_finish", "duplicate_finish"):
        raise ValueError("unknown terminal fault")
    records = [json.loads(line) for line in data.splitlines() if line.strip()]
    rows = [r for r in records if ".request." in r.get("schema_version", "") and r.get("phase") == "measure"]
    summaries = [r for r in records if ".summary." in r.get("schema_version", "")]
    if len(rows) != count or len(summaries) != 1 or summaries[0].get("attempted") != count:
        raise ValueError("request/summary population mismatch")
    groups = {}
    for event in (json.loads(line) for line in trace.splitlines() if line.strip()):
        groups.setdefault(event["request"], []).append(event)
    if len(groups) != count or len(g6_checks.server_usage(trace)) != count:
        raise ValueError("complete real server usage traces required")
    for group in groups.values():
        original = b"".join(base64.b64decode(e["bytes_b64"], validate=True) for e in group if e["kind"] == "chunk")
        forwarded = b"".join(base64.b64decode(e["bytes_b64"], validate=True) for e in group if e["kind"] == "forwarded_chunk")
        transforms = [e for e in group if e["kind"] == "transformation"]
        if not transforms or any(e.get("mode") != mode for e in transforms) or original == forwarded:
            raise ValueError("required actual-byte transformation not observed")
        original_frames = g2_usage_checks.parse_sse(original)
        forwarded_frames = g2_usage_checks.parse_sse(forwarded)
        original_finishes = [c.get("finish_reason") for f in original_frames for c in f.get("choices") or [] if c.get("finish_reason")]
        forwarded_finishes = [c.get("finish_reason") for f in forwarded_frames for c in f.get("choices") or [] if c.get("finish_reason")]
        if mode == "duplicate_finish" and (not original_finishes or len(forwarded_finishes) != 2 * len(original_finishes)):
            raise ValueError("real finish was not duplicated exactly")
        if mode == "remove_finish_and_done" and (not original_finishes or forwarded_finishes or b"[DONE]" in forwarded or b"[DONE]" not in original):
            raise ValueError("real finish and done were not removed")
        if mode == "nonlength_finish" and ("length" not in original_finishes or "length" in forwarded_finishes or "stop" not in forwarded_finishes):
            raise ValueError("real length finish was not changed to nonlength")
        if mode == "remove_output":
            for frame in forwarded_frames:
                for choice in frame.get("choices") or []:
                    delta = choice.get("delta") or {}
                    if any(delta.get(k) for k in ("content", "reasoning", "reasoning_content", "tool_calls", "function_call")):
                        raise ValueError("output not completely removed")
    if mode == "duplicate_finish":
        detail = r_checks.truncation_cell(data, trace, count, False, code)
        if detail["truncated"] != count:
            raise ValueError("duplicated finish did not count each real truncation once")
    else:
        if code == 0 or any(r.get("error") is None for r in rows) or summaries[0].get("errors") != count:
            raise ValueError("invalid terminal branches did not fail every affected request")
        if mode in ("remove_output", "nonlength_finish") and any(r.get("outcome") != "no_output_token" for r in rows):
            raise ValueError("reasoning-only nonlength or no-output classification incorrect")
        if mode == "remove_finish_and_done" and any((r.get("error") or {}).get("kind") != "stream_truncated" for r in rows):
            raise ValueError("incomplete real stream not classified as truncated")
        detail = {"attempted": count, "errors": summaries[0]["errors"], "exit_code": code}
    return {"mode": mode, **detail}
