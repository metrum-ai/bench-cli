#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Independent population/window oracle for G3 real-server evidence.

These functions consume recorded events, not fabricated benchmark outcomes.
They do not start or interrupt a server and are not a substitute for a lane.
"""
import base64
import json
import math
import statistics

import g6_checks


def finite(value):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
        raise ValueError("missing or invalid measurement")
    return float(value)


def near(actual, expected, *, absolute=0, relative=0.01):
    return abs(finite(actual) - expected) <= max(absolute, abs(expected) * relative)


def load(data, trace):
    records = [json.loads(line) for line in data.splitlines() if line.strip()]
    rows = [r for r in records if ".request." in r.get("schema_version", "") and r.get("phase") == "measure"]
    summaries = [r for r in records if ".summary." in r.get("schema_version", "")]
    if not rows or len(summaries) != 1 or summaries[0].get("attempted") != len(rows):
        raise ValueError("missing measured rows or summary")
    groups = {}
    for event in (json.loads(line) for line in trace.splitlines() if line.strip()):
        groups.setdefault(event["request"], []).append(event)
    if len(groups) != len(rows):
        raise ValueError("real proxy request coverage does not match records")
    starts, ends = [], []
    for group in groups.values():
        sent = [e["monotonic_ns"] for e in group if e["kind"] == "send"]
        terminal = [e["monotonic_ns"] for e in group if e["kind"] in ("complete", "error")]
        if len(sent) != 1 or len(terminal) != 1:
            raise ValueError("missing or duplicate real terminal timestamp")
        starts.append(sent[0]); ends.append(terminal[0])
    proxy_window = (max(ends) - min(starts)) / 1e9
    if proxy_window <= 0:
        raise ValueError("invalid independent proxy window")
    times = [(finite(r.get("send_offset_s")), finite(r.get("latency_s"))) for r in rows]
    if any(latency <= 0 for _, latency in times):
        raise ValueError("invalid measured latency")
    window = max(send + latency for send, latency in times) - min(send for send, _ in times)
    if window <= 0 or abs(proxy_window - window) > 0.05:
        raise ValueError("independent proxy and client windows differ by more than 50ms")
    if not near(summaries[0].get("window_seconds"), proxy_window, absolute=0.05, relative=0):
        raise ValueError("summary window disagrees with independent proxy clock")
    return rows, summaries[0], window, proxy_window


def completed(row):
    return row.get("error") is None or row.get("outcome") in ("reasoning_truncated", "no_output_token")


def real_stream_populations(trace):
    groups, complete = {}, set()
    for event in (json.loads(line) for line in trace.splitlines() if line.strip()):
        if event.get("kind") == "chunk":
            groups.setdefault(event["request"], []).append(base64.b64decode(event["bytes_b64"], validate=True))
        elif event.get("kind") == "complete":
            complete.add(event["request"])
    answered = truncated = 0
    for request, chunks in groups.items():
        if request not in complete:
            continue
        reasoning = visible = length = False
        for frame in b"".join(chunks).replace(b"\r\n", b"\n").split(b"\n\n"):
            data = b"\n".join(line[5:].lstrip() for line in frame.splitlines() if line.startswith(b"data:"))
            if not data or data == b"[DONE]":
                continue
            try:
                event = json.loads(data)
            except json.JSONDecodeError:
                # An interrupted final frame may be partial; it cannot prove a population.
                continue
            for choice in event.get("choices") or []:
                if choice.get("index", 0) != 0:
                    continue
                delta = choice.get("delta") or {}
                reasoning |= bool(delta.get("reasoning") or delta.get("reasoning_content"))
                visible |= bool(delta.get("content"))
                length |= choice.get("finish_reason") == "length"
        answered += int(visible)
        truncated += int(reasoning and not visible and length)
    return answered, truncated


def verify_complete_control(data, trace):
    rows, summary, window, proxy_window = load(data, trace)
    answered = [r for r in rows if r.get("answer_completed") is True and r.get("error") is None]
    truncated = [r for r in rows if r.get("outcome") == "reasoning_truncated"]
    real_answered, real_truncated = real_stream_populations(trace)
    if not answered or not truncated or real_answered < len(answered) or real_truncated < len(truncated):
        raise ValueError("complete-usage control requires observed real answers and truncations")
    if any(r.get("error") is not None or r.get("usage_prompt_tokens_present") is not True or r.get("usage_completion_tokens_present") is not True for r in rows):
        raise ValueError("complete-usage control requires complete successful usage")
    if not any(a["send_offset_s"] < t["send_offset_s"] + t["latency_s"] and t["send_offset_s"] < a["send_offset_s"] + a["latency_s"] for a in answered for t in truncated):
        raise ValueError("complete-usage control populations do not overlap")
    server = g6_checks.server_usage(trace)
    if len(server) != len(rows):
        raise ValueError("complete-usage control has incomplete real-server coverage")
    totals = {}
    for field, usage_field, summary_total, rate in (
            ("prompt_tokens", "prompt_tokens", "prompt_tokens_total", "input_tokens_per_second"),
            ("completion_tokens", "completion_tokens", "completion_tokens_total", "completion_tokens_per_second")):
        count = sum(r[field] for r in rows)
        if count != sum(r[usage_field] for r in server) or summary.get(summary_total) != count or not near(summary.get(rate), count / window):
            raise ValueError(f"complete-usage control {field} total or rate mismatch")
        totals[field] = count
    if not near(summary.get("total_tokens_per_second"), sum(totals.values()) / window):
        raise ValueError("complete-usage control total rate mismatch")
    return {"attempted": len(rows), "answered": len(answered), "truncated": len(truncated),
            "tokens": totals, "window_s": window, "proxy_window_s": proxy_window}


def verify_window(data, trace, order):
    rows, summary, window, proxy_window = load(data, trace)
    real_answered, real_truncated = real_stream_populations(trace)
    if not real_answered or not real_truncated or not any(json.loads(line).get("kind") == "error" for line in trace.splitlines() if line.strip()):
        raise ValueError("real answer, reasoning truncation and upstream interruption not observed in trace")
    answered = [r for r in rows if r.get("answer_completed") is True and r.get("error") is None]
    truncated = [r for r in rows if r.get("outcome") == "reasoning_truncated"]
    failed = [r for r in rows if r.get("error") is not None and r.get("outcome") != "reasoning_truncated"]
    if not answered or not truncated or not failed:
        raise ValueError("need real answer, reasoning truncation and interrupted/failed request")
    if real_answered < len(answered) or real_truncated < len(truncated):
        raise ValueError("CLI answer or truncation population exceeds real completed streams")
    if not any(a["send_offset_s"] < t["send_offset_s"] + t["latency_s"] and t["send_offset_s"] < a["send_offset_s"] + a["latency_s"] for a in answered for t in truncated):
        raise ValueError("answered and truncated generations did not overlap")
    first_answer = min(r["send_offset_s"] + r["latency_s"] for r in answered)
    first_fail = min(r["send_offset_s"] + r["latency_s"] for r in failed)
    if order not in ("error-first", "error-last") or (first_fail < first_answer) != (order == "error-first") or first_fail == first_answer:
        raise ValueError("required failure ordering was not observed")
    complete_usage = all(r.get("usage_completion_tokens_present") is True for r in rows)
    if complete_usage:
        total = sum(r["completion_tokens"] for r in rows)
        server_usage = g6_checks.server_usage(trace)
        if len(server_usage) != len(rows) or sum(r["completion_tokens"] for r in server_usage) != total:
            raise ValueError("independent server usage population mismatch")
        if summary.get("completion_tokens_total") != total or not near(summary.get("completion_tokens_per_second"), total / window):
            raise ValueError("completion total or rate mismatch")
    elif summary.get("completion_tokens_total") is not None or summary.get("completion_tokens_per_second") is not None:
        raise ValueError("incomplete usage must yield null aggregate")
    return {"attempted": len(rows), "answered": len(answered), "truncated": len(truncated), "failed": len(failed),
            "order": order, "client_window_s": window, "proxy_window_s": proxy_window,
            "completion_tokens_total": summary.get("completion_tokens_total"), "usage_complete": complete_usage}


def median_answered_values(rows):
    """Apply the same-cohort population requirement without computing a threshold."""
    answered = [row for row in rows if row.get('answer_completed') is True and row.get('error') is None]
    values = [finite(row.get('latency_s')) for row in answered]
    if len(values) < 2 or len(set(values)) < 2 or any(value <= 0 for value in values):
        raise ValueError('same-cohort median requires two distinct positive answered latencies')
    return values


def verify_goodput_median(data, trace):
    rows, summary, window, proxy_window = load(data, trace)
    answered = [row for row in rows if row.get('answer_completed') is True and row.get('error') is None]
    values = median_answered_values(rows)
    if any(finite(row.get('queue_delay_s', 0)) != 0 for row in rows):
        raise ValueError('same-cohort raw e2e requires zero scheduling delay')
    slo = statistics.median(values)
    if not math.isclose(finite(summary.get('goodput', {}).get('thresholds_s', {}).get('e2e')), slo, abs_tol=1e-9, rel_tol=0):
        raise ValueError('tool threshold differs from independently derived median')
    if summary.get('config', {}).get('modality', {}).get('cert_e2e_median_slo') is not True:
        raise ValueError('tool did not record same-cohort median mode')
    good = sum(value <= slo for value in values)
    if summary.get('answered_requests') != len(answered) or summary.get('goodput', {}).get('count') != good:
        raise ValueError('tool answered or goodput count differs')
    if not near(summary.get('goodput', {}).get('requests_per_second'), good / window):
        raise ValueError('tool goodput rate differs from independent count/window')
    if not any(row.get('outcome') == 'reasoning_truncated' and row.get('answer_completed') is False for row in rows):
        raise ValueError('real unanswered truncation required')
    if not any(row.get('error') is not None and row.get('outcome') != 'reasoning_truncated' for row in rows):
        raise ValueError('real failure required')
    return {'attempted': len(rows), 'answered': len(answered), 'answered_pass': good,
            'answered_fail': len(answered) - good, 'e2e_slo_s': slo,
            'window_s': window, 'proxy_window_s': proxy_window, 'goodput_requests_per_second': good / window}


def verify_goodput(data, trace, e2e_s):
    rows, summary, window, proxy_window = load(data, trace)
    e2e_s = finite(e2e_s)
    if e2e_s <= 0:
        raise ValueError("SLO must be positive")
    answered = [r for r in rows if r.get("answer_completed") is True and r.get("error") is None]
    good = [r for r in answered if finite(r.get("latency_s")) + finite(r.get("queue_delay_s", 0)) <= e2e_s]
    bad_answer = [r for r in answered if r not in good]
    truncated = [r for r in rows if r.get("outcome") == "reasoning_truncated" and r.get("answer_completed") is False]
    failed = [r for r in rows if r.get("error") is not None and r.get("outcome") != "reasoning_truncated"]
    if not good or not bad_answer or not truncated or not failed:
        raise ValueError("mixed answered SLO outcomes, truncation and real failure required")
    if summary.get("answered_requests") != len(answered) or summary.get("goodput", {}).get("count") != len(good):
        raise ValueError("answer or goodput population differs")
    for actual, expected in ((summary.get("requests_per_second"), sum(completed(row) for row in rows) / window),
                             (summary.get("answered_requests_per_second"), len(answered) / window),
                             (summary.get("goodput", {}).get("requests_per_second"), len(good) / window)):
        if not near(actual, expected):
            raise ValueError("completed, answered or SLO-goodput rate differs")
    if summary.get("goodput", {}).get("thresholds_s", {}).get("e2e") != e2e_s:
        raise ValueError("reported SLO differs from frozen threshold")
    return {"attempted": len(rows), "answered_pass": len(good), "answered_fail": len(bad_answer),
            "truncated": len(truncated), "failed": len(failed), "window_s": window, "proxy_window_s": proxy_window,
            "goodput_requests_per_second": len(good) / window}
