#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Independent strategic sweep/export checks; CPU fixtures are not GPU evidence."""
import csv
import io
import json
import math


def verify(summary_text, csv_text, trace, interruption, exit_code, export_failure_summary, export_failure_code):
    summary = json.loads(summary_text)
    rows = list(csv.DictReader(io.StringIO(csv_text)))
    points = summary.get('points')
    if exit_code == 0 or summary.get('partial') or not isinstance(points, list) or len(points) != 2:
        raise ValueError('two complete stages and nonzero safety exit are required')
    failure_points = json.loads(export_failure_summary).get('points', [])
    if (export_failure_code == 0 or len(failure_points) != 1 or failure_points[0].get('n') != 1
            or failure_points[0].get('errors') != 0 or failure_points[0].get('answered_requests') != 1):
        raise ValueError('successful real summary must survive a deliberate export failure')
    if interruption.get('kill_exit') != 0 or interruption.get('start_exit') != 0:
        raise ValueError('real restart evidence missing')
    events = [json.loads(line) for line in trace.splitlines() if line.strip()]
    start, end = interruption['kill_start_ns'], interruption['kill_end_ns']
    if not any(e.get('kind') == 'error' and start <= e.get('monotonic_ns', 0) <= end + 120_000_000_000 for e in events):
        raise ValueError('real upstream stream interruption missing')
    if not any(e.get('kind') == 'complete' and e.get('monotonic_ns', 0) < start for e in events):
        raise ValueError('completed real stream before kill missing')
    if not rows or any(r.get('warmup') not in ('false', 'true') for r in rows):
        raise ValueError('strategic CSV rows or warmup provenance missing')
    total_success = total_error = 0
    for stage, point in zip((1.0, 2.0), points):
        if point.get('load') != stage:
            raise ValueError('stage identity or order differs')
        measured = [r for r in rows if float(r['stage']) == stage and r['warmup'] == 'false']
        if not measured or point.get('n') != len(measured):
            raise ValueError('stage request population differs from CSV')
        if any(r.get('success') not in ('true', 'false') for r in measured):
            raise ValueError('invalid CSV success state')
        good_rows = [r for r in measured if r['success'] == 'true']
        failed = [r for r in measured if r['success'] == 'false']
        if any(not r.get('error') for r in failed) or point.get('errors') != len(failed):
            raise ValueError('failed CSV rows or error count missing')
        total_success += len(good_rows); total_error += len(failed)
        starts = [float(r['send_offset_s']) for r in measured]
        ends = [start + float(r['service_latency_s']) for start, r in zip(starts, measured)]
        window = max(ends) - min(starts)
        if not math.isfinite(window) or window <= 0:
            raise ValueError('stage window invalid')
        good = sum(r['success'] == 'true' and r.get('answer_completed') != 'false' and r.get('valid') != 'false' for r in measured)
        if not math.isclose(float(point.get('goodput')), good / window, rel_tol=.01, abs_tol=1e-9):
            raise ValueError('strategic goodput differs from independent CSV count/window')
    if not total_success or not total_error:
        raise ValueError('at least one successful and one failed real request required')
    return {'stages': 2, 'successes': total_success, 'errors': total_error, 'csv_rows': len(rows),
            'summary_before_failed_export': True}
