# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""External wrapper v1.8.0 field-contract checks over retained real traces."""
import json
import math
from pathlib import Path

import r_checks


FIELDS = ('ttft_s', 'ttft_visible_answer_s', 'ttft_source', 'outcome', 'answer_completed', 'usage_missing')


def option(argv, name):
    indexes = [i for i, item in enumerate(argv) if item == name]
    if len(indexes) != 1 or indexes[0] + 1 >= len(argv):
        raise ValueError(f'missing or duplicate CLI option {name}')
    return argv[indexes[0] + 1]


def evaluate(data, trace, wrapper, count, mode, exit_code, candidate, url, prompt, cli_log, model):
    if mode not in ('adequate', 'strict', 'safety'):
        raise ValueError('unknown W cell')
    if not isinstance(wrapper, dict) or not isinstance(wrapper.get('cli_argv'), list):
        raise ValueError('wrapper did not retain its candidate invocation')
    argv = wrapper['cli_argv']; cap = 4096 if mode == 'adequate' else 128
    if not argv or Path(argv[0]).resolve() != Path(candidate).resolve():
        raise ValueError('wrapper candidate invocation differs from pinned binary')
    fixed = {'--url': url, '--api-key': 'dummy', '--prompts': str(prompt), '--data-log': str(cli_log), '--model': model,
             '--num-requests': str(count), '--max-tokens': str(cap), '--mode': 'chat', '--warmup-requests': '0'}
    if (any(option(argv, key) != value for key, value in fixed.items()) or argv.count('--streaming') != 1
            or json.loads(option(argv, '--extra-body-json')) != {'chat_template_kwargs': {'enable_thinking': True}}):
        raise ValueError('wrapper candidate command differs from the frozen cell')
    for flag, required in (('--strict-reasoning-truncation', mode == 'strict'), ('--ci-safety', mode == 'safety')):
        if argv.count(flag) != int(required): raise ValueError('strict CLI flag missing or misplaced')
    cli_exit = wrapper.get('cli_exit_code')
    if (not isinstance(cli_exit, int) or isinstance(cli_exit, bool) or not isinstance(exit_code, int)
            or isinstance(exit_code, bool) or cli_exit != (0 if mode == 'adequate' else 1)
            or (mode == 'adequate' and exit_code != 0) or (mode != 'adequate' and not 1 <= exit_code <= 125)):
        raise ValueError('wrapper or candidate exit indicates a crash or incorrect strict handling')
    if mode == 'adequate':
        cell = r_checks.adequate_cell(data, trace, count, cli_exit)
    else:
        cell = r_checks.truncation_cell(data, trace, count, True, cli_exit)
    rows, summary, _ = r_checks.load_cell(data, trace, count)
    native = wrapper.get('requests'); wrapped_summary = wrapper.get('summary')
    if not isinstance(native, list) or len(native) != len(rows) or not isinstance(wrapped_summary, dict):
        raise ValueError('wrapper request or summary population missing')
    native_by_seq = {}
    for item in native:
        if not isinstance(item, dict) or not isinstance(item.get('seq'), int) or isinstance(item.get('seq'), bool) or item['seq'] in native_by_seq:
            raise ValueError('duplicate or missing wrapper request identity')
        native_by_seq[item['seq']] = item
    if len({row.get('seq') for row in rows}) != len(rows): raise ValueError('duplicate CLI request identity')
    for row in rows:
        item = native_by_seq.get(row.get('seq'))
        if not item or row.get('schema_version') != 'metrum-ai-bench-cli.request.v5':
            raise ValueError('wrapper did not consume a v5 request')
        for field in FIELDS:
            # The CLI omits its optional visible clock when there is no answer;
            # the wrapper must represent that as an explicit null.
            if field not in item or (field != 'ttft_visible_answer_s' and field not in row) or item[field] != row.get(field):
                raise ValueError(f'wrapper field mismatch: {field}')
        if row['ttft_source'] != 'stream' or row['usage_missing'] is not False:
            raise ValueError('stream clock provenance or complete usage missing')
        first = row['ttft_s']; visible = row.get('ttft_visible_answer_s')
        if not isinstance(first, (int, float)) or isinstance(first, bool) or not math.isfinite(first) or first < 0:
            raise ValueError('invalid first-any clock')
        if visible is not None and (not isinstance(visible, (int, float)) or isinstance(visible, bool) or not math.isfinite(visible) or visible < first):
            raise ValueError('invalid visible clock')
        if row['outcome'] == 'reasoning_truncated':
            if row['answer_completed'] is not False or not isinstance(item.get('warning'), str) or not item['warning'].strip():
                raise ValueError('truncation must be unanswered and warned')
    expected = {'attempted': len(rows), 'answered_requests': sum(row.get('answer_completed') is True for row in rows),
                'successes': sum(row.get('error') is None for row in rows), 'errors': sum(row.get('error') is not None for row in rows),
                'usage_missing_count': sum(row.get('usage_missing') is True for row in rows),
                'reasoning_truncated': sum(row.get('outcome') == 'reasoning_truncated' for row in rows)}
    if any(wrapped_summary.get(key) != value for key, value in expected.items()):
        raise ValueError('wrapper counts do not match CLI requests')
    if (summary.get('attempted') != expected['attempted'] or summary.get('successes') != expected['successes'] or summary.get('errors') != expected['errors']
            or summary.get('answered_requests') != expected['answered_requests']
            or summary.get('usage_missing_count') != expected['usage_missing_count']
            or summary.get('ttft_coverage', {}).get('reasoning_truncated') != expected['reasoning_truncated']):
        raise ValueError('CLI summary differs from request cohort')
    return {**cell, 'counts': expected, 'wrapper_fields_match': len(rows), 'strict_mode': mode}
