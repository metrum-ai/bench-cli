#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Independent every-rate checks on real G3-style window evidence."""
import g3_checks


def verify(data, trace, slo_s):
    rows, summary, window, proxy_window = g3_checks.load(data, trace)
    successes = [r for r in rows if r.get('error') is None]
    answered = [r for r in rows if r.get('answer_completed') is True and r.get('error') is None]
    failures = [r for r in rows if r.get('error') is not None]
    good = [r for r in answered if g3_checks.finite(r.get('latency_s')) + g3_checks.finite(r.get('queue_delay_s', 0)) <= slo_s]
    def rate(field, expected):
        actual = summary.get(field)
        if expected is None:
            if actual is not None:
                raise ValueError(f'{field} must be null')
        elif not g3_checks.near(actual, expected):
            raise ValueError(f'{field} does not match independent numerator/window')
    complete_usage = all(r.get('usage_missing') is False for r in rows)
    if any(r.get('usage_missing') is True and (r.get('tokenized_prompt_tokens') is not None or r.get('tokenized_completion_tokens') is not None) for r in rows):
        raise ValueError('independent tokenizer fallback verification is not implemented')
    prompt = sum(r['prompt_tokens'] for r in rows) if complete_usage else None
    output = sum(r['completion_tokens'] for r in rows) if complete_usage else None
    if summary.get('prompt_tokens_total') != prompt or summary.get('completion_tokens_total') != output:
        raise ValueError('all-measured token numerators differ')
    rate('input_tokens_per_second', None if prompt is None else prompt / window)
    rate('completion_tokens_per_second', None if output is None else output / window)
    rate('total_tokens_per_second', None if prompt is None or output is None else (prompt + output) / window)
    visible_complete = all(r.get('usage_missing') is False and r.get('visible_completion_tokens') is not None for r in rows)
    visible = sum(r['visible_completion_tokens'] for r in rows) if visible_complete else None
    rate('non_reasoning_completion_tokens_per_second', None if visible is None else visible / window)
    rate('requests_per_second', sum(g3_checks.completed(r) for r in rows) / window)
    rate('answered_requests_per_second', len(answered) / window)
    if summary.get('answered_requests') != len(answered) or summary.get('goodput', {}).get('count') != len(good):
        raise ValueError('answered or goodput count differs')
    if summary.get('goodput', {}).get('thresholds_s', {}).get('e2e') != slo_s or not g3_checks.near(summary.get('goodput', {}).get('requests_per_second'), len(good) / window):
        raise ValueError('goodput rate or SLO differs')
    legacy = [r for r in successes if r.get('outcome') != 'reasoning_truncated']
    legacy_window = max((r['send_offset_s'] + r['latency_s'] for r in legacy), default=0) - min(r['send_offset_s'] for r in rows)
    if any(r.get('usage_missing') is True for r in legacy):
        raise ValueError('legacy numerator incomplete')
    legacy_rate = sum(r['completion_tokens'] for r in legacy) / legacy_window if legacy and legacy_window > 0 else None
    rate('legacy_success_completion_tokens_per_second', legacy_rate)
    overlap = sum(a['send_offset_s'] < f['send_offset_s'] + f['latency_s'] and f['send_offset_s'] < a['send_offset_s'] + a['latency_s'] for a in answered for f in failures)
    return {'attempted': len(rows), 'answered': len(answered), 'failures': len(failures), 'goodput_count': len(good),
            'failure_answer_overlaps': overlap, 'usage_complete': complete_usage, 'client_window_s': window,
            'proxy_window_s': proxy_window, 'legacy_output_tokens_per_second': legacy_rate}
