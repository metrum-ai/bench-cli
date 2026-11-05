#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Check mixed serial real-server attempts against original recorded SSE bytes."""
import base64
import json


def verify(data, trace, order, strict=False, count=16):
    records = [json.loads(line) for line in data.splitlines() if line.strip()]
    rows = sorted((r for r in records if '.request.' in r.get('schema_version', '') and r.get('phase') == 'measure'), key=lambda r: r['seq'])
    summary = [r for r in records if '.summary.' in r.get('schema_version', '')]
    if count < 4 or len(rows) != count or [r.get('seq') for r in rows] != list(range(count)) or len(summary) != 1:
        raise ValueError('unique measured request slots and one summary required')
    groups = {}
    for event in (json.loads(line) for line in trace.splitlines() if line.strip()):
        groups.setdefault(event['request'], []).append(event)
    if sorted(groups) != list(range(1, count + 1)):
        raise ValueError('serial real-server request traces required')
    seen = {'answer': 0, 'reasoning_truncated': 0, 'failure': 0}
    terminals = {'answer': [], 'failure': []}
    for row, request_id in zip(rows, sorted(groups)):
        events = groups[request_id]
        transformed = [e for e in events if e.get('kind') == 'request_transform']
        send = [e for e in events if e.get('kind') == 'send']
        end = [e for e in events if e.get('kind') in ('error', 'complete')]
        if len(transformed) != 1 or transformed[0].get('forwarded_cap') != (128 if request_id % 2 else 4096) or len(send) != 1 or len(end) != 1:
            raise ValueError('real request caps or send/terminal evidence differ')
        if end[0]['kind'] == 'error':
            actual = 'failure'
        else:
            raw = b''.join(base64.b64decode(e['bytes_b64'], validate=True) for e in events if e.get('kind') == 'chunk')
            visible = reasoning = length = False
            for frame in raw.replace(b'\r\n', b'\n').split(b'\n\n'):
                payload = b'\n'.join(line[5:].lstrip() for line in frame.splitlines() if line.startswith(b'data:'))
                if not payload or payload == b'[DONE]':
                    continue
                item = json.loads(payload)
                for choice in item.get('choices') or []:
                    if choice.get('index', 0) != 0:
                        continue
                    delta = choice.get('delta') or {}
                    visible |= bool(delta.get('content'))
                    reasoning |= bool(delta.get('reasoning') or delta.get('reasoning_content'))
                    length |= choice.get('finish_reason') == 'length'
            if visible:
                actual = 'answer'
            elif reasoning and length:
                actual = 'reasoning_truncated'
            else:
                raise ValueError('unclassifiable real stream outcome')
        seen[actual] += 1
        if actual in terminals:
            terminals[actual].append(end[0]['monotonic_ns'])
        if actual == 'answer' and (row.get('answer_completed') is not True or row.get('error') is not None):
            raise ValueError('actual answer differs from CLI classification')
        if actual == 'reasoning_truncated' and (row.get('outcome') != 'reasoning_truncated' or row.get('answer_completed') is not False or (row.get('error') is not None) != strict):
            raise ValueError('actual truncation differs from CLI or strictness classification')
        if actual == 'failure' and row.get('error') is None:
            raise ValueError('real interrupted stream lacks a CLI error')
    if not all(seen.values()):
        raise ValueError('real answer, truncation and interruption populations required')
    if order not in ('error-first', 'error-last') or (min(terminals['failure']) < min(terminals['answer'])) != (order == 'error-first'):
        raise ValueError('failure ordering not observed')
    reported = summary[0]
    errors = sum(row.get('error') is not None for row in rows)
    if (reported.get('attempted') != count or reported.get('errors') != errors or reported.get('successes') != count-errors or
        reported.get('answered_requests') != seen['answer']):
        raise ValueError('per-request and summary outcome counts differ')
    return {'attempted': count, **seen, 'errors': errors, 'strict': strict, 'order': order}
