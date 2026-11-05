#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Independent type-7 distribution checks from captured per-request records."""
import math
import statistics


def _quantile(values, p):
    if not values:
        return None
    index = (len(values) - 1) * p
    low = int(index)
    high = math.ceil(index)
    return values[low] * (high - index) + values[high] * (index - low) if low != high else values[low]


def verify(summary, field, values):
    values = sorted(float(x) for x in values if x is not None and math.isfinite(float(x)))
    actual = summary.get(field)
    if not isinstance(actual, dict) or actual.get('n') != len(values) or actual.get('percentile_method') != 'hyndman_fan_type7':
        raise ValueError(f'{field} distribution population or method differs')
    median = _quantile(values, .5)
    expected = {'min': values[0] if values else None, 'max': values[-1] if values else None,
                'avg': statistics.mean(values) if values else None,
                'std': statistics.stdev(values) if len(values) >= 2 else None,
                'mad': _quantile(sorted(abs(x - median) for x in values), .5) if values else None,
                'p50': median, 'p90': _quantile(values, .9), 'p95': _quantile(values, .95), 'p99': _quantile(values, .99)}
    for key, value in expected.items():
        got = actual.get(key)
        if (value is None and got is not None) or (value is not None and (isinstance(got, bool) or not isinstance(got, (int, float)) or not math.isfinite(got) or not math.isclose(got, value, rel_tol=.01, abs_tol=1e-6))):
            raise ValueError(f'{field}.{key} differs from independent per-request distribution')
    return len(values)
