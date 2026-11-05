#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Independent live modality and strategic checks; CPU fixtures are not GPU evidence."""
import csv
import io
import json
import math

import g1_clock_checks
import g3_checks
import g4_rates_checks
import g6_checks
import independent_distributions
import independent_timeweighted
import schema_checks


def verify_images(trace, expected_sha, model):
    identities = [json.loads(line) for line in trace.splitlines() if line.strip() and json.loads(line).get('kind') == 'image_identity']
    sends = [json.loads(line) for line in trace.splitlines() if line.strip() and json.loads(line).get('kind') == 'send']
    if not identities or len(identities) != len(sends) or {r['request'] for r in identities} != {r['request'] for r in sends}:
        raise ValueError('every real VLM request requires recorded image identity')
    if any(row.get('image_sha256') != [expected_sha] or row.get('model') != model or row.get('thinking') is not True for row in identities):
        raise ValueError('VLM request image, model or thinking setting differs from pins')
    return len(identities)


def verify_distributions(summary, rows):
    coverage = summary.get('ttft_coverage', {})
    visible = sum(row.get('ttft_visible_answer_s') is not None for row in rows)
    expected = {'measured': len(rows), 'visible': visible, 'missing_visible': len(rows) - visible,
                'reasoning_only': sum(row.get('first_reasoning_s') is not None and row.get('ttft_visible_answer_s') is None for row in rows),
                'reasoning_truncated': sum(row.get('outcome') == 'reasoning_truncated' for row in rows),
                'stream': sum(row.get('ttft_source') == 'stream' for row in rows),
                'first_token_fallback': sum(row.get('ttft_source') == 'first_token_fallback' for row in rows),
                'first_byte_approx': sum(row.get('ttft_source') == 'first_byte_approx' for row in rows),
                'unresolved': sum(row.get('ttft_source') is None for row in rows)}
    if any(coverage.get(key) != value for key, value in expected.items()):
        raise ValueError('TTFT timing cohorts differ from captured request records')
    fields = ('latency_s', 'ttft_s', 'ttft_first_any_s', 'ttft_visible_answer_s', 'first_reasoning_s', 'decode_s', 'prefill_s', 'decode_tok_s')
    for field in fields:
        independent_distributions.verify(summary, field, [row.get(field) for row in rows])
    prefill, users, decode = [], [], []
    for row in rows:
        first, latency = row.get('ttft_s'), row['latency_s']
        if first is not None and row.get('first_reasoning_s') is not None:
            first = min(first, row['first_reasoning_s'])
        if first is not None and first > 0 and row['prompt_tokens'] > 0 and row.get('ttft_source') != 'first_byte_approx':
            prefill.append(row['prompt_tokens'] / first)
        if latency > 0 and row['completion_tokens'] > 0:
            users.append(row['completion_tokens'] / latency)
        ttft = row.get('ttft_s')
        if ttft is not None and latency > ttft and row['completion_tokens'] > 0:
            decode.append(row['completion_tokens'] / (latency - ttft))
    independent_distributions.verify(summary, 'prefill_tps_per_user', prefill)
    independent_distributions.verify(summary, 'user_tps', users)
    independent_distributions.verify(summary, 'decode_tok_s', decode)
    second, intervals, tpot = [], [], []
    for row in rows:
        itls = row.get('itl_s') or []
        intervals.extend(itls)
        visible = row.get('ttft_visible_answer_s')
        if visible is None and row.get('first_reasoning_s') is None and row.get('ttft_source') != 'first_byte_approx':
            visible = row.get('ttft_s')
        if visible is not None and itls:
            second.append(visible + itls[0])
        ttft = row.get('ttft_s')
        if ttft is not None and row['latency_s'] > ttft and row['completion_tokens'] > 1:
            tpot.append((row['latency_s'] - ttft) / (row['completion_tokens'] - 1))
    independent_distributions.verify(summary, 'time_to_second_token_s', second)
    if 'itl_s' in summary: independent_distributions.verify(summary, 'itl_s', intervals)
    if 'tpot_s' in summary: independent_distributions.verify(summary, 'tpot_s', tpot)


def strategic_rates(summary_text, csv_text, trace):
    summary = json.loads(summary_text)
    points = summary.get('points')
    rows = [r for r in csv.DictReader(io.StringIO(csv_text)) if r.get('warmup') == 'false']
    if not isinstance(points, list) or len(points) != 1 or not rows or points[0].get('n') != len(rows):
        raise ValueError('one strategic stage with complete measured CSV rows required')
    point = points[0]
    if point.get('config', {}).get('warmup_requests') != 0 or any(row.get('warmup') != 'false' for row in rows):
        raise ValueError('strategic warmup must be disabled and excluded')
    if any(row.get('success') != 'true' or row.get('usage_missing') != 'false' or float(row.get('queue_delay_s', 'nan')) != 0 for row in rows) or point.get('errors') != 0:
        raise ValueError('strategic modality comparison requires complete successful usage')
    server = g6_checks.server_usage(trace)
    if not any(item['reasoning_seen'] for item in server):
        raise ValueError('strategic real reasoning stream missing')
    if len(server) != len(rows) or sum(int(r['input_tokens']) for r in rows) != sum(r['prompt_tokens'] for r in server) or sum(int(r['output_tokens']) for r in rows) != sum(r['completion_tokens'] for r in server):
        raise ValueError('strategic CSV token counts differ from real server usage')
    starts = [float(r['send_offset_s']) for r in rows]
    ends = [start + float(row['service_latency_s']) for start, row in zip(starts, rows)]
    window = max(ends) - min(starts)
    if not math.isfinite(window) or window <= 0:
        raise ValueError('invalid strategic measured window')
    answered = sum(row.get('answer_completed') == 'true' for row in rows)
    completed = len(rows)
    good = sum(row.get('answer_completed') != 'false' and row.get('valid') != 'false' for row in rows)
    prompt = sum(int(row['input_tokens']) for row in rows)
    output = sum(int(row['output_tokens']) for row in rows)
    if not answered or point.get('answered_requests') != answered:
        raise ValueError('strategic answered cohort differs')
    legacy = [row for row in rows if row.get('outcome') != 'reasoning_truncated']
    legacy_window = max((float(row['send_offset_s']) + float(row['service_latency_s']) for row in legacy), default=0) - min(starts)
    legacy_rate = sum(int(row['output_tokens']) for row in legacy) / legacy_window if legacy and legacy_window > 0 else None
    has_split = all(row.get('reasoning_tokens', '').isdigit() and int(row['reasoning_tokens']) <= int(row['output_tokens']) for row in rows)
    visible_rate = sum(int(row['output_tokens']) - int(row['reasoning_tokens']) for row in rows) / window if has_split else None
    for field, expected in (('throughput', completed / window), ('answered_requests_per_second', answered / window),
                            ('goodput', good / window), ('input_tokens_per_second', prompt / window),
                            ('completion_tokens_per_second', output / window), ('total_tokens_per_second', (prompt + output) / window),
                            ('legacy_success_completion_tokens_per_second', legacy_rate), ('non_reasoning_completion_tokens_per_second', visible_rate)):
        actual = point.get(field)
        if (expected is None and actual is not None) or (expected is not None and not g3_checks.near(actual, expected)):
            raise ValueError(f'strategic {field} disagrees with independent CSV numerator/window')
    spans = []
    for row in rows:
        first = float(row['ttft_s']) if row.get('ttft_s') and row.get('ttft_source') != 'first_byte_approx' else None
        if first is not None and row.get('first_reasoning_s'):
            first = min(first, float(row['first_reasoning_s']))
        spans.append({'start': float(row['send_offset_s']), 'latency': float(row['service_latency_s']), 'first': first,
                      'prompt': int(row['input_tokens']), 'output': int(row['output_tokens'])})
    independent_timeweighted.verify(point, spans, min(starts), window)
    normalized = []
    for row in rows:
        normalized.append({key: (float(row[key]) if row.get(key) else None) for key in
            ('ttft_s', 'ttft_first_any_s', 'ttft_visible_answer_s', 'first_reasoning_s', 'decode_s', 'prefill_s', 'decode_tok_s')} |
            {'latency_s': float(row['service_latency_s']), 'prompt_tokens': int(row['input_tokens']),
             'completion_tokens': int(row['output_tokens']), 'ttft_source': row.get('ttft_source') or None,
             'outcome': row.get('outcome') or None,
             'itl_s': [float(value) for value in row.get('itl_s', '').split(';') if value]})
    verify_distributions(point, normalized)
    streams = g1_clock_checks.stream_clocks(trace)
    if len(streams) != len(rows):
        raise ValueError('strategic real-stream coverage incomplete')
    for row, stream in zip(sorted(rows, key=lambda r: float(r['send_offset_s'])), streams):
        if abs(float(row['service_latency_s']) - (stream['finish'] - stream['send']) / 1e9) > .05:
            raise ValueError('strategic client/proxy latency differs by more than 50ms')
        if stream['first'] is None or not row.get('ttft_s') or abs(float(row['ttft_s']) - (stream['first'] - stream['send']) / 1e9) > .05:
            raise ValueError('strategic first-any clock differs from real trace')
    return {'requests': len(rows), 'answered': answered, 'window_s': window}


def evaluate(llm_data, llm_trace, llm_telemetry, llm_strategic, vlm_data, vlm_trace, vlm_telemetry, vlm_strategic, image_sha, model):
    result = {}
    for name, data, trace, telemetry, strategic in (('llm', llm_data, llm_trace, llm_telemetry, llm_strategic),
                                                     ('vlm', vlm_data, vlm_trace, vlm_telemetry, vlm_strategic)):
        rows, _, _, _ = g3_checks.load(data, trace)
        server = g6_checks.server_usage(trace)
        if not any(row.get('answer_completed') is True and row.get('error') is None for row in rows) or not any(x['reasoning_seen'] for x in server):
            raise ValueError(f'{name} requires an actual reasoning stream and measured answer')
        if len(server) != len(rows) or sum(r['prompt_tokens'] for r in rows) != sum(r['prompt_tokens'] for r in server) or sum(r['completion_tokens'] for r in rows) != sum(r['completion_tokens'] for r in server):
            raise ValueError(f'{name} server usage differs from records')
        rates = g4_rates_checks.verify(data, trace, 600)
        g1_clock_checks.evaluate(data, trace, 0)
        summary = next(json.loads(line) for line in data.splitlines() if line.strip() and '.summary.' in json.loads(line).get('schema_version', ''))
        if summary.get('config', {}).get('common', {}).get('warmup_requests') != 0:
            raise ValueError('modality warmup must be disabled')
        if any(row.get('scheduled_offset_s') is not None for row in rows):
            raise ValueError('modality comparison requires closed-loop request issuance')
        width = summary.get('config', {}).get('common', {}).get('throughput_bin_seconds')
        if not isinstance(width, (float, int)) or width <= 0:
            raise ValueError('throughput bin width missing')
        window = rates['client_window_s']
        bins = [0] * max(1, math.ceil(window / width))
        first = min(row['send_offset_s'] for row in rows)
        for row in rows:
            index = math.floor((row['send_offset_s'] - first) / width)
            if index < len(bins): bins[index] += 1
        bin_rates = [count / min(width, window - i * width) for i, count in enumerate(bins)]
        independent_distributions.verify(summary, 'throughput_bins_rps', bin_rates)
        spans = []
        for row in rows:
            if row.get('error') is not None:
                continue
            first = row.get('ttft_s') if row.get('ttft_source') != 'first_byte_approx' else None
            if first is not None and row.get('first_reasoning_s') is not None:
                first = min(first, row['first_reasoning_s'])
            spans.append({'start': row['send_offset_s'], 'latency': row['latency_s'], 'first': first,
                          'prompt': row['prompt_tokens'], 'output': row['completion_tokens']})
        independent_timeweighted.verify(summary, spans, min(row['send_offset_s'] for row in rows), rates['client_window_s'])
        verify_distributions(summary, rows)
        exports = schema_checks.evaluate(data, telemetry, trace, strategic['summary'], strategic['csv'], strategic['telemetry'])
        stage = strategic_rates(strategic['summary'], strategic['csv'], strategic['trace'])
        result[name] = {'rates': rates, 'exports': exports, 'strategic': stage}
    result['vlm_images'] = verify_images(vlm_trace, image_sha, model)
    result['vlm_strategic_images'] = verify_images(vlm_strategic['trace'], image_sha, model)
    return result
