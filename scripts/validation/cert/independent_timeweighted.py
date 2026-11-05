#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Independent interval-integration oracle for time-weighted rate blocks."""
import math


def _block(segments, count, window):
    if not count or window <= 0:
        return {'n': count, 'avg': None, 'active_avg': None, 'max': None, 'active_s': None}
    clipped = [(max(0.0, start), min(window, end), level, slope, start)
               for start, end, level, slope in segments if min(window, end) > max(0.0, start)]
    points = sorted({time for seg in clipped for time in seg[:2]})
    integral = active = peak = 0.0
    for left, right in zip(points, points[1:]):
        open_segments = [seg for seg in clipped if seg[0] <= left and seg[1] >= right]
        if not open_segments:
            continue
        value_left = sum(level + slope * (left - origin) for _, _, level, slope, origin in open_segments)
        value_right = sum(level + slope * (right - origin) for _, _, level, slope, origin in open_segments)
        integral += (value_left + value_right) * (right - left) / 2
        active += right - left
        peak = max(peak, value_left, value_right)
    return {'n': count, 'avg': integral / window, 'active_avg': integral / active if active else None,
            'max': peak, 'active_s': active}


def calculate(spans, origin, window):
    if not math.isfinite(window) or window <= 0:
        raise ValueError('positive finite measurement window required')
    blocks = {key: [] for key in ('effective_concurrency', 'effective_prefill_concurrency', 'effective_decode_concurrency',
                                   'tokens_in_flight', 'effective_prefill_throughput', 'effective_decode_throughput')}
    counts = {key: 0 for key in blocks}
    for row in spans:
        start, latency = row['start'] - origin, row['latency']
        if not math.isfinite(start) or not math.isfinite(latency) or latency < 0:
            raise ValueError('invalid request interval')
        end = start + latency
        blocks['effective_concurrency'].append((start, end, 1, 0)); counts['effective_concurrency'] += 1
        first = row.get('first')
        if first is None:
            continue
        first = min(max(first, 0), latency)
        split = start + first
        for name, lo, hi in (('effective_prefill_concurrency', start, split), ('effective_decode_concurrency', split, end)):
            blocks[name].append((lo, hi, 1, 0)); counts[name] += 1
        prompt, output = row.get('prompt'), row.get('output')
        if prompt is not None and output is not None:
            blocks['tokens_in_flight'].extend([(start, split, prompt, 0), (split, end, prompt, output / (latency-first) if latency > first else 0)])
            counts['tokens_in_flight'] += 1
        if prompt is not None and prompt > 0 and first > 0:
            blocks['effective_prefill_throughput'].append((start, split, prompt / first, 0)); counts['effective_prefill_throughput'] += 1
        if output is not None and output > 0 and latency > first:
            blocks['effective_decode_throughput'].append((split, end, output / (latency-first), 0)); counts['effective_decode_throughput'] += 1
    return {name: _block(segments, counts[name], window) for name, segments in blocks.items()}


def verify(summary, spans, origin, window):
    expected = calculate(spans, origin, window)
    for name, block in expected.items():
        actual = summary.get(name)
        if not isinstance(actual, dict) or actual.get('n') != block['n']:
            raise ValueError(f'{name} sample count differs')
        for field in ('avg', 'active_avg', 'max', 'active_s'):
            x, y = actual.get(field), block[field]
            if (y is None and x is not None) or (y is not None and (isinstance(x, bool) or not isinstance(x, (float, int)) or not math.isfinite(x) or not math.isclose(x, y, rel_tol=.01, abs_tol=1e-6))):
                raise ValueError(f'{name}.{field} differs from independent interval integration')
    return expected
