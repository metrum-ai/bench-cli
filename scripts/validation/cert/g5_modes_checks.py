#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Real streaming/nonstreaming byte and token-clock checks."""
import base64
import json
import math


def evaluate(data, trace, streaming, infer_first_byte, model):
    records = [json.loads(line) for line in data.splitlines() if line.strip()]
    rows = sorted((r for r in records if '.request.' in r.get('schema_version', '') and r.get('phase') == 'measure'), key=lambda r: r['seq'])
    summaries = [r for r in records if '.summary.' in r.get('schema_version', '')]
    if len(rows) != 4 or [r.get('seq') for r in rows] != list(range(4)) or len(summaries) != 1 or summaries[0].get('successes') != 4:
        raise ValueError('four unique successful rows and one matching summary required')
    groups = {}
    for e in (json.loads(line) for line in trace.splitlines() if line.strip()):
        groups.setdefault(e['request'], []).append(e)
    if sorted(groups) != list(range(1, 5)):
        raise ValueError('four real serial request traces required')
    identities, answers, tokens = [], 0, 0
    for row, rid in zip(rows, sorted(groups)):
        events = groups[rid]
        identity = [e for e in events if e.get('kind') == 'request_identity']
        if (len(identity) != 1 or identity[0].get('thinking') is not False or identity[0].get('max_tokens') != 128 or
            identity[0].get('model') != model or identity[0].get('temperature') != 0 or identity[0].get('streaming') is not streaming):
            raise ValueError('pinned request identity missing or configuration differs')
        identities.append(identity[0]['prompt_sha256'])
        if (sum(e.get('kind') == 'complete' for e in events) != 1 or
            any(e.get('kind') == 'error' for e in events) or
            not any(e.get('kind') == 'headers' and e.get('status') == 200 for e in events)):
            raise ValueError('real response did not complete successfully')
        raw = b''.join(base64.b64decode(e['bytes_b64'], validate=True) for e in events if e.get('kind') == 'chunk')
        if streaming:
            items = []
            for frame in raw.replace(b'\r\n', b'\n').split(b'\n\n'):
                payload = b'\n'.join(line[5:].lstrip() for line in frame.splitlines() if line.startswith(b'data:'))
                if payload and payload != b'[DONE]':
                    items.append(json.loads(payload))
            usages = [item['usage'] for item in items if isinstance(item.get('usage'), dict)]
            visible = any(bool((choice.get('delta') or {}).get('content')) for item in items for choice in item.get('choices') or [] if choice.get('index', 0) == 0)
            if len(usages) != 1:
                raise ValueError('stream usage missing or ambiguous')
            usage = usages[0]
            if row.get('ttft_source') != 'stream' or row.get('ttft_s') is None:
                raise ValueError('real stream token clock missing')
        else:
            item = json.loads(raw)
            usage = item.get('usage')
            choices = item.get('choices') or []
            visible = bool(choices and (choices[0].get('message') or {}).get('content'))
            if any(row.get(k) is not None for k in ('first_reasoning_s', 'ttft_first_any_s', 'ttft_visible_answer_s')):
                raise ValueError('nonstream fabricated a token timestamp')
            if infer_first_byte:
                first, ttft = row.get('first_byte_s'), row.get('ttft_s')
                if row.get('ttft_source') != 'first_byte_approx' or not isinstance(first, (float, int)) or not isinstance(ttft, (float, int)) or not math.isclose(first, ttft, abs_tol=1e-9):
                    raise ValueError('HTTP first-byte opt-in source/clock differs')
            elif row.get('ttft_s') is not None or row.get('ttft_source') is not None:
                raise ValueError('nonstream fabricated TTFT without opt-in')
        if not isinstance(usage, dict) or row.get('usage_prompt_tokens_present') is not True or row.get('usage_completion_tokens_present') is not True:
            raise ValueError('full server usage missing')
        for key in ('prompt_tokens', 'completion_tokens'):
            if not isinstance(usage.get(key), int) or isinstance(usage[key], bool) or usage[key] < 0 or row.get(key) != usage[key]:
                raise ValueError('recorded per-request usage differs from server')
        if row.get('error') is not None or row.get('answer_completed') is not visible:
            raise ValueError('client answer outcome differs from actual server output')
        answers += visible
        tokens += usage['completion_tokens']
    if not answers:
        raise ValueError('no real visible answer')
    return {'attempted': 4, 'answers': answers, 'completion_tokens': tokens, 'prompt_sha256': identities,
            'mode': 'streaming' if streaming else 'nonstreaming', 'first_byte_opt_in': infer_first_byte}
