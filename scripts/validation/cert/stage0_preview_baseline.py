#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Draft P1/P2 baseline evidence checker, NOT a preview-stop classifier.

This read-only helper does not establish a candidate fence, controller stop,
container/GPU identity, elapsed monotonic time, or cleanup. It must not be used
alone to grant a Stage-0 barrier. G7 has a different layout and is unsupported.
"""
import datetime as dt
import json
import math
from pathlib import Path
import re

import execution_evidence
import manifest_tool
import perf_evidence_checks
import stage0_milestones

SHA = re.compile(r'[a-f0-9]{64}\Z')


def _object(path):
    value = json.loads(path.read_text(encoding='utf-8'), object_pairs_hook=manifest_tool.unique_object)
    if not isinstance(value, dict):
        raise ValueError('expected JSON object')
    return value


def verify_p1p2(root, *, baseline, baseline_sha256, prompt, prompt_sha256, model):
    """Check one first baseline cell against frozen inputs, with no side effects."""
    root = Path(root).resolve(strict=True)
    if (not SHA.fullmatch(baseline_sha256) or not SHA.fullmatch(prompt_sha256)
            or not isinstance(model, str) or not model):
        raise ValueError('invalid baseline, prompt or model pin')
    binary, prompts = Path(baseline), Path(prompt)
    if (not binary.is_absolute() or binary.is_symlink() or not binary.is_file()
            or not prompts.is_absolute() or prompts.is_symlink() or not prompts.is_file()
            or execution_evidence.digest(binary) != baseline_sha256
            or execution_evidence.digest(prompts) != prompt_sha256):
        raise ValueError('baseline or prompt pin differs')
    base = 'bench-rel/1/W1/c1'
    for suffix, duration in (('warmup', 30), ('', 180)):
        folder = f'{base}/{suffix}' if suffix else base
        stage0_milestones.local_file(root, f'{folder}/DONE')
        point = _object(stage0_milestones.local_file(root, f'{folder}/point.json'))
        command = _object(stage0_milestones.local_file(root, f'{folder}/command.json'))
        if (point.get('tool') != 'bench-rel' or point.get('workload') != 'W1'
                or type(point.get('concurrency')) is not int or point['concurrency'] != 1
                or point.get('pass') != '1'):
            raise ValueError('baseline point identity differs')
        times = [point.get('start'), point.get('stop')]
        if (any(type(x) not in (int, float) or not math.isfinite(x) for x in times)
                or times[1] - times[0] < duration):
            raise ValueError('baseline wall duration too short')
        if (command.get('binary_sha256') != baseline_sha256 or command.get('binary_sha256_end') != baseline_sha256
                or command.get('prompt_sha256') != prompt_sha256 or command.get('prompt_sha256_end') != prompt_sha256):
            raise ValueError('baseline input hashes differ')
        argv = command.get('argv')
        if not isinstance(argv, list) or not argv or argv[0] != str(binary):
            raise ValueError('baseline argv missing or wrong binary')
        required = {'--scenario': 'W1', '--concurrency': '1', '--model': model,
                    '--url': 'http://127.0.0.1:8000/v1/chat/completions', '--api-key': 'dummy',
                    '--max-tokens': '128', '--temperature': '0', '--seed': '42',
                    '--num-requests': '1000000', '--stop-after-seconds': str(duration),
                    '--mode': 'chat', '--prompts': str(prompts),
                    '--data-log': str(root / folder / 'requests.jsonl')}
        if (any(perf_evidence_checks.option(argv, name) != expected for name, expected in required.items())
                or argv.count('--streaming') != 1
                or json.loads(perf_evidence_checks.option(argv, '--extra-body-json'), object_pairs_hook=manifest_tool.unique_object) != {'seed': 42}):
            raise ValueError('baseline command differs from frozen spec')
    path = stage0_milestones.local_file(root, f'{base}/requests.jsonl')
    completed = False
    with path.open(encoding='utf-8') as stream:
        for line in stream:
            if not line.strip():
                continue
            row = json.loads(line, object_pairs_hook=manifest_tool.unique_object)
            if (isinstance(row, dict) and re.fullmatch(r'metrum-ai-bench-cli\.request\.v[0-9]+', str(row.get('schema_version', '')))
                    and row.get('phase') == 'measure' and row.get('error') is None
                    and row.get('outcome') == 'answer' and row.get('answer_completed') is True):
                try:
                    start = dt.datetime.fromisoformat(row['started_at'].replace('Z', '+00:00'))
                    end = dt.datetime.fromisoformat(row['completed_at'].replace('Z', '+00:00'))
                    completed = (start.tzinfo is not None and end.tzinfo is not None and end >= start
                                 and times[0] <= start.timestamp() <= end.timestamp() <= times[1])
                except (KeyError, TypeError, AttributeError, ValueError):
                    continue
                if completed:
                    break
    if not completed:
        raise ValueError('no completed baseline request')
    # Freeze the source bytes again after reading the evidence.
    if execution_evidence.digest(binary) != baseline_sha256 or execution_evidence.digest(prompts) != prompt_sha256:
        raise ValueError('baseline inputs changed during verification')
    return {'baseline_cell': 'bench-rel/1/W1/c1', 'warmup_seconds': 30, 'measure_seconds': 180}
