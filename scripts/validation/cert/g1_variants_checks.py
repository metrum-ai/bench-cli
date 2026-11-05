# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Independent forwarded-clock and source-byte provenance checks for GPU-003."""
import base64
import json

import g1_clock_checks
import sse_reorder
from trace_proxy import take_frame


def frames(data):
    output = []
    while data:
        frame, data = take_frame(data)
        if frame is None:
            raise ValueError('unterminated retained stream')
        output.append(frame)
    return output


def verify_source_and_collect(trace, mode):
    groups = {}
    for line in trace.splitlines():
        if line.strip():
            event = json.loads(line)
            groups.setdefault(event['request'], []).append(event)
    if not groups:
        raise ValueError('no source requests')
    populations = {'role': 0, 'reasoning': 0, 'any_reasoning': 0, 'visible': 0, 'tool': 0, 'unselected_before_selected': 0,
                   'answer_before_reasoning': 0, 'same_event': 0}
    for events in groups.values():
        for required in ('send', 'headers', 'complete'):
            if sum(event['kind'] == required for event in events) != 1:
                raise ValueError('missing or duplicate stream evidence: ' + required)
        if any(event['kind'] == 'error' for event in events):
            raise ValueError('stream recorded an error')
        if next(event['status'] for event in events if event['kind'] == 'headers') != 200:
            raise ValueError('source server did not return HTTP 200')
        original = b''.join(base64.b64decode(e['bytes_b64'], validate=True) for e in events if e['kind'] == 'chunk')
        forwarded = b''.join(base64.b64decode(e['bytes_b64'], validate=True) for e in events if e['kind'] == 'forwarded_chunk')
        source = frames(original)
        actual = frames(forwarded)
        transformations = [event for event in events if event['kind'] == 'transformation']
        if mode is None:
            if original != forwarded or transformations:
                raise ValueError('untransformed stream does not match real source')
        else:
            if len(transformations) != 1 or transformations[0].get('mode') != mode:
                raise ValueError('missing or ambiguous transform marker')
            if actual != sse_reorder.transform(source, mode):
                raise ValueError('forwarded stream differs from deterministic real-byte transformation')
        if not source or sse_reorder.decode(source[-1]) is not None:
            raise ValueError('real source lacks DONE')
        # Independently check each source choice's output text is preserved in order.
        def contents(sequence):
            output = {}
            for frame in sequence:
                item = sse_reorder.decode(frame)
                for pos, choice in enumerate((item or {}).get('choices') or []):
                    idx = choice.get('index', pos)
                    delta = choice.get('delta') or {}
                    for key in ('content', 'reasoning', 'reasoning_content'):
                        value = delta.get(key)
                        if isinstance(value, str) and value:
                            output.setdefault((idx, key), []).append(value)
            return output
        if contents(source) != contents(actual):
            raise ValueError('transformation changed or dropped model-generated deltas')
        # Process actual forwarded timing, not the upstream receive clock.
        pending = b''
        role_at = reason_at = visible_at = unselected_at = None
        tool_seen = False
        for event in events:
            if event['kind'] != 'forwarded_chunk':
                continue
            pending += base64.b64decode(event['bytes_b64'], validate=True)
            while True:
                frame, pending = take_frame(pending)
                if frame is None:
                    break
                item = sse_reorder.decode(frame)
                if item is None:
                    continue
                for pos, choice in enumerate(item.get('choices') or []):
                    idx = choice.get('index', pos)
                    delta = choice.get('delta') or {}
                    reason = any(isinstance(delta.get(k), str) and delta[k] for k in ('reasoning', 'reasoning_content'))
                    content = delta.get('content') if isinstance(delta.get('content'), str) else choice.get('text')
                    answer = isinstance(content, str) and bool(content)
                    fn = delta.get('function_call') or {}
                    tool = isinstance(fn, dict) and isinstance(fn.get('arguments'), str) and bool(fn['arguments'])
                    for call in delta.get('tool_calls') or []:
                        fn = call.get('function') or {}
                        tool |= isinstance(fn.get('arguments'), str) and bool(fn['arguments'])
                    when = event['monotonic_ns']
                    if idx != 0:
                        if idx == 1 and (reason or answer or tool) and unselected_at is None: unselected_at = when
                        continue
                    if delta.get('role') and role_at is None: role_at = when
                    if reason and reason_at is None: reason_at = when
                    if (answer or tool) and visible_at is None: visible_at = when
                    tool_seen |= tool
        if pending or visible_at is None:
            raise ValueError('missing complete selected answer/tool output')
        first = min(x for x in (reason_at, visible_at) if x is not None)
        populations['role'] += role_at is not None and role_at < first
        populations['reasoning'] += reason_at is not None and reason_at < visible_at
        populations['any_reasoning'] += reason_at is not None
        populations['visible'] += visible_at is not None
        populations['tool'] += tool_seen
        populations['unselected_before_selected'] += unselected_at is not None and unselected_at < first
        populations['answer_before_reasoning'] += reason_at is not None and visible_at < reason_at
        populations['same_event'] += reason_at is not None and visible_at == reason_at
    return populations


def evaluate(data, trace, code, mode, required):
    timing = g1_clock_checks.evaluate(data, trace, code, 'forwarded_chunk', require_reasoning=False)
    populations = verify_source_and_collect(trace, mode)
    if required != 'answer_only' and not populations.get(required):
        raise ValueError('required real/transformed event population missing: ' + required)
    if not populations['role']:
        raise ValueError('role-only frame before selected output missing')
    if required == 'answer_only' and (not populations['visible'] or populations['any_reasoning']):
        raise ValueError('answer-only cell contains reasoning or no answer')
    return {'timing': timing, 'populations': populations, 'transform': mode}
