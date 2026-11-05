# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Prospectively reorder/merge actual model-generated SSE deltas for GPU-003.

This is a fault transformation, not a server. Original bytes must be retained;
missing source deltas, unsupported frames or ambiguous multi-choice events fail.
"""
import copy
import json

MODES = ('answer-first', 'same-event', 'unselected-first')


def decode(frame):
    normalized = frame.replace(b'\r\n', b'\n')
    if not normalized.endswith(b'\n\n'):
        raise ValueError('incomplete original SSE frame')
    lines = normalized[:-2].split(b'\n')
    if len(lines) != 1 or not lines[0].startswith(b'data:'):
        raise ValueError('unsupported SSE framing')
    payload = lines[0][5:].lstrip()
    if payload == b'[DONE]':
        return None
    data = json.loads(payload)
    if not isinstance(data, dict):
        raise ValueError('non-object SSE payload')
    return data


def selected_delta(event):
    if event is None:
        return None
    choices = event.get('choices') or []
    if not isinstance(choices, list):
        raise ValueError('invalid choices')
    if len(choices) > 1:
        raise ValueError('ambiguous multi-choice reorder')
    if not choices:
        return None
    choice = choices[0]
    if not isinstance(choice, dict) or choice.get('index', 0) != 0:
        raise ValueError('reorder requires selected choice only')
    delta = choice.get('delta') or {}
    if not isinstance(delta, dict):
        raise ValueError('invalid delta')
    return delta


def transform(frames, mode):
    if mode not in MODES or not frames:
        raise ValueError('unknown or empty transformation')
    events = [decode(frame) for frame in frames]
    if events[-1] is not None or sum(event is None for event in events) != 1:
        raise ValueError('source stream must finish with exactly one DONE')
    if mode == 'unselected-first':
        selected_index = unselected_index = None
        for i, event in enumerate(events):
            if event is None:
                continue
            choices = event.get('choices') or []
            if len(choices) > 1:
                raise ValueError('ambiguous batched multi-choice event')
            if not choices:
                continue
            choice = choices[0]
            delta = choice.get('delta') or {}
            if not isinstance(delta, dict):
                raise ValueError('invalid multi-choice delta')
            token = any(isinstance(delta.get(key), str) and delta[key] for key in ('content', 'reasoning', 'reasoning_content'))
            if token and choice.get('index') == 0 and selected_index is None:
                selected_index = i
            if token and choice.get('index') == 1 and unselected_index is None:
                unselected_index = i
        if selected_index is None or unselected_index is None or selected_index == unselected_index:
            raise ValueError('real source lacks selected and unselected output')
        choice = events[unselected_index]['choices'][0]
        if choice.get('finish_reason') is not None or 'role' in (choice.get('delta') or {}):
            raise ValueError('cannot move unselected terminal or role metadata')
        out = list(frames)
        if selected_index < unselected_index:
            unselected = out.pop(unselected_index)
            out.insert(selected_index, unselected)
        return out
    reason_index = answer_index = None
    reason_key = None
    for i, event in enumerate(events):
        delta = selected_delta(event)
        if delta is None:
            continue
        keys = [key for key in ('reasoning_content', 'reasoning') if isinstance(delta.get(key), str) and delta[key]]
        answer = delta.get('content')
        if len(keys) > 1 or keys and isinstance(answer, str) and answer:
            raise ValueError('source frame has ambiguous output fields')
        if reason_index is None and keys:
            reason_index, reason_key = i, keys[0]
        if answer_index is None and isinstance(answer, str) and answer:
            answer_index = i
    if reason_index is None or answer_index is None or reason_index >= answer_index:
        raise ValueError('real source must contain reasoning before a visible answer')
    answer_choice = events[answer_index]['choices'][0]
    if answer_choice.get('finish_reason') is not None or 'role' in (answer_choice.get('delta') or {}):
        raise ValueError('cannot move output frame with terminal or role metadata')
    if mode == 'answer-first':
        out = list(frames)
        answer = out.pop(answer_index)
        out.insert(reason_index, answer)
        return out
    reason = events[reason_index]
    answer = copy.deepcopy(events[answer_index])
    reason_delta = selected_delta(reason)
    answer_delta = selected_delta(answer)
    reason_choice = reason['choices'][0]
    if (set(reason_delta) != {reason_key} or reason_key in answer_delta
            or any(value is not None for key, value in reason_choice.items() if key not in ('index', 'delta'))
            or any(key not in answer or answer[key] != value for key, value in reason.items() if key != 'choices')):
        raise ValueError('cannot safely combine source deltas and metadata')
    answer_delta[reason_key] = reason_delta[reason_key]
    newline = b'\r\n' if b'\r\n' in frames[answer_index] else b'\n'
    merged = b'data: ' + json.dumps(answer, separators=(',', ':'), ensure_ascii=False).encode('utf-8') + newline + newline
    # Drop the single-field reasoning frame and move the merged real deltas to
    # its position. Every other frame retains its exact original bytes/order.
    out = [frame for i, frame in enumerate(frames) if i not in (reason_index, answer_index)]
    out.insert(reason_index, merged)
    return out
