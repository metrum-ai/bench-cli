#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Validate selective timeout/malformed faults against two real requests."""
import base64
import json
import math


def verify(data, trace, fault, target, exit_code, timeout_s=2, count=2):
    records = [json.loads(line) for line in data.splitlines() if line.strip()]
    rows = sorted((r for r in records if '.request.' in r.get('schema_version', '') and r.get('phase') == 'measure'), key=lambda r: r['seq'])
    summary = [r for r in records if '.summary.' in r.get('schema_version', '')]
    if count < 2 or target not in (1, count) or len(rows) != count or [r.get('seq') for r in rows] != list(range(count)) or len(summary) != 1 or exit_code == 0:
        raise ValueError('ordered measured slots, one summary and nonzero safety exit required')
    failed = rows[target - 1]
    good = [row for index, row in enumerate(rows, 1) if index != target]
    expected = 'timeout' if fault == 'timeout' else 'parse_error'
    if (failed.get('error') or {}).get('kind') != expected or failed.get('answer_completed') is True:
        raise ValueError('failure classification or incomplete outcome differs')
    if any(row.get('error') is not None or row.get('answer_completed') is not True for row in good):
        raise ValueError('control request did not answer')
    if summary[0].get('attempted') != count or summary[0].get('successes') != count - 1 or summary[0].get('errors') != 1 or summary[0].get('errors_by_type', {}).get(expected) != 1:
        raise ValueError('summary failed to count success and fault once')
    groups = {}
    for event in (json.loads(line) for line in trace.splitlines() if line.strip()):
        groups.setdefault(event['request'], []).append(event)
    if sorted(groups) != list(range(1, count + 1)):
        raise ValueError('distinct real request trace for every measured request required')
    events = groups[target]
    if not any(e.get('kind') == 'chunk' for e in events):
        raise ValueError('fault did not originate with real server bytes')
    if fault == 'timeout':
        latency = failed.get('latency_s')
        if isinstance(latency, bool) or not isinstance(latency, (int, float)) or not math.isfinite(latency) or latency > timeout_s + 5:
            raise ValueError('timeout exceeded configured timeout plus 5 seconds')
        if not any(e.get('kind') == 'delay' and e.get('seconds', 0) > timeout_s for e in events):
            raise ValueError('real response delay evidence is missing')
    elif fault == 'corrupt':
        if sum(e.get('kind') == 'transformation' and e.get('mode') == 'corrupt' for e in events) != 1:
            raise ValueError('one real malformed frame transformation required')
        original = b''.join(base64.b64decode(e['bytes_b64'], validate=True) for e in events if e.get('kind') == 'chunk')
        forwarded = b''.join(base64.b64decode(e['bytes_b64'], validate=True) for e in events if e.get('kind') == 'forwarded_chunk')
        if not original or not forwarded or original == forwarded:
            raise ValueError('original/forwarded malformed evidence missing')
    else:
        raise ValueError('unknown fault')
    if any(e.get('kind') in ('delay', 'transformation') for request, events in groups.items() if request != target for e in events):
        raise ValueError('fault unexpectedly applied to successful control')
    return {'fault': fault, 'failure_slot': target - 1, 'success_slot': 1 if target == 1 else 0,
            'successful_controls': count - 1, 'error_kind': expected,
            'failure_latency_s': failed.get('latency_s'), 'exit_code': exit_code}
