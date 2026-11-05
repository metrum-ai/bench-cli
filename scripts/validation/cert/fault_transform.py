#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Transform actual server SSE frames; never fabricate generation or usage.

Callers must retain both original and forwarded bytes plus the selected mode.
An untouched frame is passed through exactly, including its line endings.
"""
import json

MODES = ("remove_usage", "remove_reasoning_split", "remove_finish", "duplicate_finish", "nonlength_finish", "corrupt")


def transform(frame, mode):
    if mode not in MODES:
        raise ValueError("unknown fault mode")
    newline = b"\r\n" if b"\r\n" in frame else b"\n"
    lines = frame.split(newline)
    data_lines = [line[5:].lstrip() for line in lines if line.startswith(b"data:")]
    if not data_lines or b"\n".join(data_lines) == b"[DONE]":
        return frame, False
    try:
        event = json.loads(b"\n".join(data_lines))
    except (ValueError, UnicodeError):
        return frame, False
    if not isinstance(event, dict):
        return frame, False
    if mode == "corrupt":
        # Truncate an actual JSON payload; this is not a synthetic response.
        data = b"\n".join(data_lines)
        return b"data: " + data[:max(1, len(data) // 2)] + newline + newline, True
    changed = False
    if mode == "remove_usage" and isinstance(event.get("usage"), dict):
        del event["usage"]
        changed = True
    elif mode == "remove_reasoning_split":
        usage = event.get("usage")
        details = usage.get("completion_tokens_details") if isinstance(usage, dict) else None
        if isinstance(details, dict) and "reasoning_tokens" in details:
            del details["reasoning_tokens"]
            changed = True
    elif mode in ("remove_finish", "duplicate_finish", "nonlength_finish"):
        choices = event.get("choices")
        if not isinstance(choices, list):
            return frame, False
        for choice in choices:
            if not isinstance(choice, dict) or choice.get("finish_reason") is None:
                continue
            if mode == "remove_finish":
                choice["finish_reason"] = None
                changed = True
            elif mode == "nonlength_finish" and choice["finish_reason"] == "length":
                choice["finish_reason"] = "stop"
                changed = True
            elif mode == "duplicate_finish":
                return frame + frame, True
    if not changed:
        return frame, False
    # Preserve non-data SSE fields (including comments, ids and event names).
    remaining = [line for line in lines if line and not line.startswith(b"data:")]
    payload = json.dumps(event, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
    return newline.join([*remaining, b"data: " + payload]) + newline + newline, True
