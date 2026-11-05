#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Trace-aligned reference comparison; never relabel reference first-choice TTFT."""
import base64
import json
import math
import statistics

import g6_checks


def finite(value):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
        raise ValueError("missing/non-finite measurement")
    return float(value)


def traces(text, order, model, cap):
    groups = {}
    for event in (json.loads(line) for line in text.splitlines() if line.strip()):
        groups.setdefault(event["request"], []).append(event)
    if len(groups) != len(order):
        raise ValueError("trace count differs from frozen prompt sequence")
    streams = []
    for index, request in enumerate(sorted(groups)):
        events = groups[request]
        ids = [e for e in events if e["kind"] == "request_identity"]
        sent = [e["monotonic_ns"] for e in events if e["kind"] == "send"]
        end = [e["monotonic_ns"] for e in events if e["kind"] == "complete"]
        if len(ids) != 1 or len(sent) != 1 or len(end) != 1 or any(e["kind"] == "error" for e in events):
            raise ValueError("missing identity, send, completion or real stream error")
        identity = ids[0]
        if (identity.get("prompt_sha256") != order[index].get("prompt_sha256") or
            identity.get("model") != model or identity.get("max_tokens") != cap or isinstance(identity.get("max_tokens"), bool) or
            identity.get("temperature") != 0 or identity.get("seed") != 42 or identity.get("thinking") is not True):
            raise ValueError("prompt order or frozen request settings differ")
        buf = b""; choices = first_any = None; reasoning = visible = False
        for event in events:
            if event["kind"] != "chunk":
                continue
            buf += base64.b64decode(event["bytes_b64"], validate=True).replace(b"\r\n", b"\n")
            while b"\n\n" in buf:
                frame, buf = buf.split(b"\n\n", 1)
                payload = b"\n".join(line[5:].lstrip() for line in frame.splitlines() if line.startswith(b"data:"))
                if not payload or payload == b"[DONE]":
                    continue
                item = json.loads(payload)
                items = item.get("choices") or []
                if items and choices is None:
                    choices = event["monotonic_ns"]
                for choice in items:
                    if choice.get("index", 0) != 0:
                        continue
                    delta = choice.get("delta") or {}
                    reason = bool(delta.get("reasoning_content") or delta.get("reasoning"))
                    answer = bool(delta.get("content"))
                    reasoning |= reason; visible |= answer
                    if first_any is None and (reason or answer):
                        first_any = event["monotonic_ns"]
        if choices is None or first_any is None or not visible:
            raise ValueError("real first-choice/first-any or answer missing")
        streams.append({"send": sent[0], "end": end[0], "first_choice_s": (choices - sent[0]) / 1e9,
                        "first_any_s": (first_any - sent[0]) / 1e9, "reasoning": reasoning})
    usage = g6_checks.server_usage(text)
    if len(usage) != len(order):
        raise ValueError("independent server usage incomplete")
    return streams, usage


def observed_clock_gaps(text):
    """Report partial real-stream gaps even if full case qualification fails."""
    groups = {}
    for event in (json.loads(line) for line in text.splitlines() if line.strip()):
        groups.setdefault(event["request"], []).append(event)
    gaps = []
    for events in groups.values():
        buffer = b""; first_choice = first_any = None
        for event in events:
            if event.get("kind") != "chunk":
                continue
            buffer += base64.b64decode(event["bytes_b64"], validate=True).replace(b"\r\n", b"\n")
            while b"\n\n" in buffer:
                frame, buffer = buffer.split(b"\n\n", 1)
                payload = b"\n".join(line[5:].lstrip() for line in frame.splitlines() if line.startswith(b"data:"))
                if not payload or payload == b"[DONE]":
                    continue
                item = json.loads(payload)
                choices = item.get("choices") or []
                if choices and first_choice is None:
                    first_choice = event["monotonic_ns"]
                for choice in choices:
                    if choice.get("index", 0) != 0:
                        continue
                    delta = choice.get("delta") or {}
                    if first_any is None and any(delta.get(k) for k in ("reasoning_content", "reasoning", "content")):
                        first_any = event["monotonic_ns"]
        if first_choice is not None and first_any is not None:
            gaps.append((first_choice - first_any) / 1e9)
    if not gaps:
        return {"paired_streams": 0}
    return {"paired_streams": len(gaps), "first_choices_minus_first_any_median_s": statistics.median(gaps),
            "first_choices_minus_first_any_p95_s": percentile95(gaps)}


def percentile95(values):
    return sorted(values)[math.ceil(.95 * len(values)) - 1]


def compare(candidate_data, candidate_trace, reference_data, reference_trace, order, model, cap):
    if len(order) != 64 or [r.get("seq") for r in order] != list(range(64)):
        raise ValueError("64 frozen prompt slots required")
    cstreams, cusage = traces(candidate_trace, order, model, cap)
    rstreams, rusage = traces(reference_trace, order, model, cap)
    if not any(s["reasoning"] for s in cstreams) or not any(s["reasoning"] for s in rstreams):
        raise ValueError("thinking-on runs did not observe actual reasoning")
    rows = [json.loads(line) for line in candidate_data.splitlines() if line.strip()]
    requests = sorted((r for r in rows if ".request." in r.get("schema_version", "") and r.get("phase") == "measure"), key=lambda r: r["seq"])
    summaries = [r for r in rows if ".summary." in r.get("schema_version", "")]
    if len(requests) != 64 or [r.get("seq") for r in requests] != list(range(64)) or len(summaries) != 1 or summaries[0].get("attempted") != 64:
        raise ValueError("candidate measured rows or summary missing")
    if any(r.get("error") is not None or r.get("answer_completed") is not True or
           r.get("usage_prompt_tokens_present") is not True or r.get("usage_completion_tokens_present") is not True for r in requests):
        raise ValueError("candidate must answer all 64 with full usage")
    if reference_data.get("completed") != 64 or reference_data.get("failed") != 0:
        raise ValueError("reference did not complete all 64")
    output_lens, input_lens = reference_data.get("output_lens"), reference_data.get("input_lens")
    ref_ttft, texts, errors = reference_data.get("ttfts"), reference_data.get("generated_texts"), reference_data.get("errors")
    if any(not isinstance(x, list) or len(x) != 64 for x in (output_lens, input_lens, ref_ttft, texts, errors)):
        raise ValueError("reference detailed 64-request evidence missing")
    if any(not isinstance(t, str) or not t.strip() for t in texts) or any(errors):
        raise ValueError("reference must answer all 64")
    if any(r["completion_tokens"] != u["completion_tokens"] or r["prompt_tokens"] != u["prompt_tokens"] for r, u in zip(requests, cusage)):
        raise ValueError("candidate per-request token usage differs from real server")
    if any(out != u["completion_tokens"] or inp != u["prompt_tokens"] for out, inp, u in zip(output_lens, input_lens, rusage)):
        raise ValueError("reference per-request token usage differs from real server")
    if any(abs(finite(row.get("ttft_s")) - stream["first_any_s"]) > .05 for row, stream in zip(requests, cstreams)):
        raise ValueError("candidate first-any does not match its own proxy trace within 50ms")
    if any(abs(finite(ttft) - stream["first_choice_s"]) > .05 for ttft, stream in zip(ref_ttft, rstreams)):
        raise ValueError("reference first-choice does not match its own proxy trace within 50ms")
    cw = (max(s["end"] for s in cstreams) - min(s["send"] for s in cstreams)) / 1e9
    rw = (max(s["end"] for s in rstreams) - min(s["send"] for s in rstreams)) / 1e9
    cwindow, rwindow = finite(summaries[0].get("window_seconds")), finite(reference_data.get("duration"))
    if min(cw, rw, cwindow, rwindow) <= 0 or abs(cwindow - cw) > .05 or abs(rwindow - rw) > max(.05, rw * .01):
        raise ValueError("candidate/reference measured-window definitions are not aligned")
    ctotal = sum(u["completion_tokens"] for u in cusage); rtotal = sum(u["completion_tokens"] for u in rusage)
    cr, rr = finite(summaries[0].get("completion_tokens_per_second")), finite(reference_data.get("output_throughput"))
    if ctotal != summaries[0].get("completion_tokens_total") or rtotal != reference_data.get("total_output_tokens") or not math.isclose(cr, ctotal / cwindow, rel_tol=.01) or not math.isclose(rr, rtotal / rwindow, rel_tol=.01):
        raise ValueError("native output throughput differs from matched server numerators and windows")
    if cr <= 0 or rr <= 0:
        raise ValueError("non-positive throughput")
    ref_p50 = statistics.median(map(finite, ref_ttft)); choice_p50 = statistics.median(s["first_choice_s"] for s in rstreams)
    ttft_diff = abs(ref_p50 - choice_p50); ttft_limit = max(.05, .2 * choice_p50)
    delta = 100 * (cr - rr) / rr
    gaps = [s["first_choice_s"] - s["first_any_s"] for s in rstreams]
    return {"requests_per_tool": 64, "candidate_reasoning_streams": sum(s["reasoning"] for s in cstreams),
            "reference_reasoning_streams": sum(s["reasoning"] for s in rstreams),
            "candidate_completion_tokens": ctotal, "reference_completion_tokens": rtotal,
            "candidate_output_tokens_per_second": cr, "reference_output_tokens_per_second": rr,
            "throughput_delta_pct": delta, "candidate_window_s": cwindow, "reference_window_s": rwindow,
            "reference_native_ttft_p50_s": ref_p50, "proxy_first_choices_p50_s": choice_p50,
            "proxy_first_any_p50_s": statistics.median(s["first_any_s"] for s in rstreams),
            "first_choices_minus_first_any_median_s": statistics.median(gaps),
            "first_choices_minus_first_any_p95_s": percentile95(gaps),
            "reference_ttft_alignment_limit_s": ttft_limit, "reference_ttft_aligned": ttft_diff <= ttft_limit,
            "throughput_within_10_percent": abs(delta) <= 10,
            "criteria_status": "prospective r3 approved; development execution is not release certification"}
