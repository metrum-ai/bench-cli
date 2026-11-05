# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Independent success-cohort reconstruction for 27B performance evidence."""
import datetime as dt
import json
import math
import statistics
from pathlib import Path


def number(value, positive=False):
    if (not isinstance(value, (int, float)) or isinstance(value, bool) or not math.isfinite(value)
            or (positive and value <= 0)):
        raise ValueError('missing or invalid finite measurement')
    return float(value)


def point(folder):
    folder = Path(folder)
    if not (folder / 'DONE').is_file():
        raise ValueError('point process did not complete')
    with (folder / 'requests.jsonl').open(encoding='utf-8') as stream:
        records = [json.loads(line) for line in stream if line.strip()]
    summary = [row for row in records if '.summary.' in row.get('schema_version', '')]
    requests = [row for row in records if '.request.' in row.get('schema_version', '') and row.get('phase') == 'measure']
    if len(summary) != 1 or summary[0].get('partial') is True or not requests or summary[0].get('attempted') != len(requests):
        raise ValueError('incomplete summary or measured request population')
    run_id = (summary[0].get('config') or {}).get('run_id') or summary[0].get('run_id')
    if not run_id or any(row.get('run_id') != run_id for row in requests):
        raise ValueError('request/summary run identity mismatch')
    if any(row.get('first_reasoning_s') is not None for row in requests):
        raise ValueError('thinking observed in thinking-off gate')
    offsets = [row.get('send_offset_s') for row in requests]
    if all(value is None for value in offsets):
        starts = []
        for row in requests:
            time = dt.datetime.fromisoformat(row['started_at'].replace('Z', '+00:00'))
            if time.tzinfo is None: raise ValueError('send timestamp lacks timezone')
            starts.append(time.timestamp())
    elif any(value is None for value in offsets):
        raise ValueError('mixed send-clock sources')
    else:
        starts = [number(value) for value in offsets]
    successes, ends, tpot, output = [], [], [], 0
    for row, start in zip(requests, starts):
        if row.get('error') is not None or row.get('outcome') == 'reasoning_truncated':
            continue
        latency = number(row.get('latency_s'), positive=True)
        tokens, prompt_tokens = row.get('completion_tokens'), row.get('prompt_tokens')
        if (not isinstance(tokens, int) or isinstance(tokens, bool) or tokens < 0
                or not isinstance(prompt_tokens, int) or isinstance(prompt_tokens, bool) or prompt_tokens < 0
                or row.get('usage_missing')):
            raise ValueError('legacy success usage missing')
        first = row.get('ttft_s')
        if tokens > 1 and first is not None:
            first = number(first)
            if first < 0: raise ValueError('negative first token clock')
            if latency > first: tpot.append((latency - first) / (tokens - 1) * 1000)
        successes.append(row); ends.append(start + latency); output += tokens
    if not successes or not tpot or output <= 0:
        raise ValueError('empty comparable success population')
    window = max(ends) - min(starts)
    if not math.isfinite(window) or window <= 0:
        raise ValueError('invalid success-only legacy window')
    totals = {}
    for key in ('completion_tokens', 'prompt_tokens'):
        values = [row.get(key) for row in requests]
        totals[key] = sum(values) if all(isinstance(v, int) and not isinstance(v, bool) and v >= 0 for v in values) else None
    return {'attempted': len(requests), 'successes': len(successes), 'measured_numerators': totals, 'legacy_output_tokens': output,
            'legacy_window_seconds': window, 'legacy_output_tok_s': output / window,
            'legacy_tpot_p50_ms': statistics.median(tpot),
            'native_headline': {key: summary[0].get(key) for key in ('attempted', 'successes', 'errors', 'ttft_coverage',
                                'window_seconds', 'completion_tokens_total', 'prompt_tokens_total',
                                'usage_missing_count', 'completion_tokens_per_second', 'ttft_first_any_s',
                                'legacy_success_completion_tokens_per_second')}}


def verify_grid(root, report, model, noise=None):
    """Recompute both releases' cohorts and the frozen per-point gate."""
    root = Path(root); noise = noise or {}
    rows = report.get('points')
    if not isinstance(rows, list) or len(rows) != 6:
        raise ValueError('incomplete perf comparison grid')
    result = []
    for workload in ('W1', 'W2'):
        for concurrency in (1, 8, 32):
            matched = [row for row in rows if row.get('workload') == workload and row.get('concurrency') == concurrency]
            if len(matched) != 1: raise ValueError('missing or duplicate comparison point')
            reported = matched[0]; observed = {}
            for name, label in (('baseline', 'bench-rel'), ('candidate', 'bench-src')):
                value = point(root / label / '1' / workload / f'c{concurrency}')
                expected = reported.get(name) or {}
                for metric, field in (('legacy_output_tok_s', 'system_output_tok_s'), ('legacy_tpot_p50_ms', 'tpot_p50_ms'),
                                      ('legacy_window_seconds', 'tool_wall_time_s'), ('legacy_output_tokens', 'output_tokens'),
                                      ('successes', 'sample_count')):
                    if not math.isclose(number(value[metric]), number(expected.get(field)), rel_tol=1e-9, abs_tol=1e-9):
                        raise ValueError('independent legacy reconstruction differs from comparator')
                if name == 'candidate':
                    headline = value['native_headline']
                    if expected.get('headline') != headline:
                        raise ValueError('candidate native headline differs from its raw summary')
                    window = number(headline.get('window_seconds'), positive=True)
                    total = number(headline.get('completion_tokens_total'), positive=True)
                    if (headline.get('attempted') != value['attempted'] or headline.get('successes') != value['successes']
                            or headline.get('completion_tokens_total') != value['measured_numerators']['completion_tokens']
                            or headline.get('prompt_tokens_total') != value['measured_numerators']['prompt_tokens']
                            or not math.isclose(number(headline.get('completion_tokens_per_second')), total / window, rel_tol=1e-9, abs_tol=1e-9)):
                        raise ValueError('candidate native numerator, window or throughput is inconsistent')
                observed[name] = value
            checks = {}
            for metric, key in (('legacy_output_tok_s', 'system_output_tok_s'), ('legacy_tpot_p50_ms', 'tpot_p50_ms')):
                baseline = number(observed['baseline'][metric], positive=True)
                candidate = number(observed['candidate'][metric], positive=True)
                floor = number(noise.get(model, {}).get(workload, {}).get(f'c{concurrency}', {}).get(key, 0))
                if floor < 0: raise ValueError('negative noise floor')
                tolerance = max(5.0, floor); delta = 100.0 * (candidate - baseline) / baseline
                expected = (reported.get('checks') or {}).get(key) or {}
                if (not math.isclose(number(expected.get('delta_pct')), delta, rel_tol=1e-9, abs_tol=1e-9)
                        or not math.isclose(number(expected.get('tolerance_pct')), tolerance, rel_tol=0, abs_tol=1e-9)
                        or expected.get('pass') is not (abs(delta) <= tolerance)):
                    raise ValueError('independent gate differs from comparator')
                checks[key] = {'delta_pct': delta, 'tolerance_pct': tolerance, 'pass': abs(delta) <= tolerance}
            result.append({'workload': workload, 'concurrency': concurrency, **observed, 'checks': checks})
    return {'points': result, 'result': 'PASS' if all(all(x['pass'] for x in row['checks'].values()) for row in result) else 'FAIL'}
