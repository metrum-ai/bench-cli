# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Strict independent scalar oracle. No label summation or physical inference."""
import math

from metrics_oracle import SAMPLE, parse_labels


def snapshot(text, specs):
    types, values, seen = {}, {}, set()
    if len(text.encode()) > 4 * 1024 * 1024:
        raise ValueError('exporter page exceeds limit')
    for line in text.splitlines():
        if line.strip() == '# EOF':
            break
        if line.startswith('# TYPE '):
            parts = line.split()
            if len(parts) != 4:
                raise ValueError('malformed TYPE')
            name, kind = parts[2:]
            relevant = [n for n in specs if n == name or n.removesuffix('_total') == name]
            for metric in relevant:
                if metric in types or metric in seen:
                    raise ValueError('late or conflicting TYPE')
                types[metric] = kind
        elif line.strip() and not line.startswith('#'):
            match = SAMPLE.fullmatch(line.strip())
            if match is None:
                if any(line.startswith(name) for name in specs):
                    raise ValueError('malformed required sample')
                continue
            name = match[1]
            if name not in specs:
                continue
            kind, unit = specs[name]
            if types.get(name) != kind:
                raise ValueError('required TYPE absent or incompatible')
            labels = parse_labels(match[2])
            key = (name, labels)
            value = float(match[3])
            if key in values or not math.isfinite(value) or value < 0:
                raise ValueError('duplicate or invalid sample')
            if unit == 'ratio' and value > 1:
                raise ValueError('ratio outside [0,1]')
            if kind == 'counter' and (value > 2**53 or (unit != 'seconds' and not value.is_integer())):
                raise ValueError('unsafe cumulative value')
            values[key] = value
            seen.add(name)
    if set(specs) - seen:
        raise ValueError('required family missing')
    return values


def deltas(before, after, specs):
    a, b = snapshot(before, specs), snapshot(after, specs)
    if a.keys() != b.keys():
        raise ValueError('worker/series identity changed')
    result = []
    for key in a:
        name, labels = key
        kind, unit = specs[name]
        if kind == 'counter' and b[key] < a[key]:
            raise ValueError('counter reset')
        result.append({'metric': name, 'labels': dict(labels), 'unit': unit,
                       'start': a[key], 'end': b[key],
                       'delta': b[key] - a[key] if kind == 'counter' else None})
    return result


def single(rows, name):
    selected = [row for row in rows if row['metric'] == name]
    if len(selected) != 1 or selected[0]['delta'] is None:
        raise ValueError('one attributable counter series required')
    return selected[0]['delta']
