#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Fail-closed independent checks for GPU-010; fixtures are not GPU evidence."""
import base64
import json
import math

import g2_usage_checks
import r_checks


def load(data):
    values = [json.loads(line) for line in data.splitlines() if line.strip()]
    def finite(value):
        if isinstance(value, float) and not math.isfinite(value):
            raise ValueError('non-finite output')
        if isinstance(value, dict):
            for child in value.values(): finite(child)
        elif isinstance(value, list):
            for child in value: finite(child)
    finite(values)
    rows = [r for r in values if '.request.' in r.get('schema_version', '') and r.get('phase') == 'measure']
    summaries = [r for r in values if '.summary.' in r.get('schema_version', '')]
    if len(summaries) != 1 or summaries[0].get('attempted') != len(rows):
        raise ValueError('missing or inconsistent summary')
    return rows, summaries[0]


def all_truncated(data, trace, code, count):
    detail = r_checks.truncation_cell(data, trace, count, False, code)
    rows, summary = load(data)
    if detail['truncated'] != count or any(r.get('answer_completed') is not False for r in rows):
        raise ValueError('all measured streams must be real unanswered truncations')
    if summary.get('answered_requests') != 0 or summary.get('legacy_success_completion_tokens_per_second') is not None:
        raise ValueError('all-truncated answer or legacy success population wrong')
    return detail


def empty(data, trace, code):
    rows, summary = load(data)
    if code != 0 or rows or any(line.strip() for line in trace.splitlines()):
        raise ValueError('zero-issuance run must have no real POST and exit successfully')
    if summary.get('window_seconds') != 0 or any(summary.get(k) is not None for k in
        ('requests_per_second', 'answered_requests_per_second', 'completion_tokens_per_second', 'input_tokens_per_second',
         'total_tokens_per_second', 'legacy_success_completion_tokens_per_second')):
        raise ValueError('zero-window rates must be null')
    if summary.get('goodput', {}).get('requests_per_second') is not None:
        raise ValueError('zero-window goodput must be null')
    return {'attempted': 0, 'window_s': 0}


def missing(data, trace, code, count, fallback):
    rows, summary = load(data)
    if code != 0 or len(rows) != count or any(row.get('error') is not None or row.get('answer_completed') is not True for row in rows):
        raise ValueError('missing-usage cell requires all real successful answers')
    groups = {}
    for event in (json.loads(line) for line in trace.splitlines() if line.strip()):
        groups.setdefault(event['request'], []).append(event)
    if len(groups) != count:
        raise ValueError('missing-usage traces are incomplete')
    for events in groups.values():
        original = b''.join(base64.b64decode(e['bytes_b64'], validate=True) for e in events if e['kind'] == 'chunk')
        forwarded = b''.join(base64.b64decode(e['bytes_b64'], validate=True) for e in events if e['kind'] == 'forwarded_chunk')
        if not original or not forwarded or not any(e.get('kind') == 'complete' for e in events):
            raise ValueError('real completed server response required')
        if not any(isinstance(f.get('usage'), dict) for f in g2_usage_checks.parse_sse(original)):
            raise ValueError('real server usage missing before transformation')
        if any(isinstance(f.get('usage'), dict) for f in g2_usage_checks.parse_sse(forwarded)) or not any(
                e.get('kind') == 'transformation' and e.get('mode') == 'remove_usage' for e in events):
            raise ValueError('real usage removal not demonstrated')
    if any(r.get('usage_completion_tokens_present') is not False or r.get('usage_prompt_tokens_present') is not False for r in rows):
        raise ValueError('missing-usage flags differ from forwarded streams')
    if any(summary.get(k) is not None for k in ('completion_tokens_total', 'completion_tokens_per_second', 'input_tokens_per_second', 'total_tokens_per_second')):
        raise ValueError('incomplete server usage must keep total rates null')
    if summary.get('completion_tokens_source') is not None:
        raise ValueError('server-only total must not claim a token source')
    if fallback:
        tokens = [r.get('tokenized_completion_tokens') for r in rows]
        if any(not isinstance(n, int) or isinstance(n, bool) or n <= 0 for n in tokens):
            raise ValueError('pinned tokenizer fallback did not produce positive counts')
        starts = [r.get('send_offset_s') for r in rows]
        ends = [start + row['latency_s'] for start, row in zip(starts, rows)]
        window = max(ends) - min(starts)
        rate = summary.get('legacy_success_completion_tokens_per_second')
        if window <= 0 or not isinstance(rate, (int, float)) or not math.isclose(rate, sum(tokens) / window, rel_tol=.01):
            raise ValueError('legacy fallback rate differs from captured tokenized counts and success window')
        return {'requests': count, 'tokenized_completion_tokens': sum(tokens), 'legacy_window_s': window, 'legacy_rate': rate}
    if summary.get('legacy_success_completion_tokens_per_second') is not None:
        raise ValueError('legacy rate must be null without complete tokenizer fallback')
    return {'requests': count, 'legacy_rate': None}
