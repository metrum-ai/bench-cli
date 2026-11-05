# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""CPU-side evidence checks for the unchanged 27B performance point grid.

These checks do not launch a server or issue a case result. A release-specific
baseline decision and both model bundles are still required by the lane.
"""
import json
import math
from pathlib import Path
import re


def option(argv, name):
    positions = [i for i, value in enumerate(argv) if value == name]
    if len(positions) != 1 or positions[0] + 1 >= len(argv):
        raise ValueError(f'missing or duplicate {name}')
    return argv[positions[0] + 1]


def verify(root, model, candidate_hash, baseline_hash, prompt_hashes, recorded_root=None, prompt_paths=None):
    root = Path(root)
    recorded_root = Path(recorded_root) if recorded_root is not None else root
    checked = []
    for workload, cap in (('W1', 128), ('W2', 512)):
        for concurrency in (1, 8, 32):
            for label, expected in (('bench-rel', baseline_hash), ('bench-src', candidate_hash)):
                directory = root / label / '1' / workload / f'c{concurrency}'
                for phase, duration in (('warmup', 30), ('measure', 180)):
                    folder = directory / 'warmup' if phase == 'warmup' else directory
                    if not (folder / 'DONE').is_file():
                        raise ValueError('missing completed performance phase')
                    point = json.loads((folder / 'point.json').read_text(encoding='utf-8'))
                    command = json.loads((folder / 'command.json').read_text(encoding='utf-8'))
                    if (point.get('tool') != label or point.get('workload') != workload
                            or point.get('concurrency') != concurrency or point.get('pass') != '1'):
                        raise ValueError('performance phase identity mismatch')
                    times = [point.get('start'), point.get('stop')]
                    if any(not isinstance(t, (int, float)) or isinstance(t, bool) or not math.isfinite(t) for t in times) or times[1] - times[0] < duration:
                        raise ValueError('performance phase wall duration is too short')
                    if (command.get('binary_sha256') != expected or command.get('binary_sha256_end') != expected
                            or command.get('prompt_sha256') != prompt_hashes[workload]
                            or command.get('prompt_sha256_end') != prompt_hashes[workload]):
                        raise ValueError('performance binary or input hash mismatch')
                    argv = command.get('argv')
                    if not isinstance(argv, list):
                        raise ValueError('missing command provenance')
                    fixed = {'--scenario': workload, '--concurrency': str(concurrency), '--model': model,
                             '--url': 'http://127.0.0.1:8000/v1/chat/completions', '--api-key': 'dummy',
                             '--max-tokens': str(cap), '--temperature': '0', '--seed': '42', '--num-requests': '1000000',
                             '--stop-after-seconds': str(duration), '--mode': 'chat'}
                    if (any(option(argv, key) != value for key, value in fixed.items()) or argv.count('--streaming') != 1
                            or (prompt_paths is not None and Path(option(argv, '--prompts')).resolve() != Path(prompt_paths[workload]).resolve())
                            or json.loads(option(argv, '--extra-body-json')) != {'seed': 42}):
                        raise ValueError('performance configuration differs from frozen spec')
                    if phase == 'measure':
                        sequence = folder / 'prompt-sequence.jsonl'
                        recorded_sequence = recorded_root / label / '1' / workload / f'c{concurrency}' / 'prompt-sequence.jsonl'
                        if option(argv, '--prompt-sequence-log') != str(recorded_sequence) or not sequence.is_file() or sequence.stat().st_size == 0:
                            raise ValueError('measured prompt sequence provenance missing')
                checked.append(f'{label}/{workload}/c{concurrency}')
    return {'verified_phases': len(checked) * 2, 'verified_tool_points': len(checked)}


def verify_sequences(root):
    """Compare effective prompt fingerprints for every issued slot, not just the seed."""
    root = Path(root); checks = []
    for workload in ('W1', 'W2'):
        for concurrency in (1, 8, 32):
            folders = [root / label / '1' / workload / f'c{concurrency}' for label in ('bench-rel', 'bench-src')]
            maxima = []
            for folder in folders:
                with (folder / 'requests.jsonl').open(encoding='utf-8') as stream:
                    records = [json.loads(line) for line in stream if line.strip()]
                summaries = [row for row in records if '.summary.' in row.get('schema_version', '') and row.get('partial') is not True]
                requests = [row for row in records if '.request.' in row.get('schema_version', '') and row.get('phase') == 'measure']
                seqs = [row.get('seq') for row in requests]
                if (len(summaries) != 1 or not seqs or len(set(seqs)) != len(seqs)
                        or any(not isinstance(seq, int) or isinstance(seq, bool) or seq < 0 for seq in seqs)
                        or summaries[0].get('attempted') != len(seqs)):
                    raise ValueError('measured request slots or summary coverage missing')
                maxima.append(max(seqs))
            with (folders[0] / 'prompt-sequence.jsonl').open(encoding='utf-8') as a, \
                 (folders[1] / 'prompt-sequence.jsonl').open(encoding='utf-8') as b:
                for index in range(max(maxima) + 1):
                    left, right = a.readline(), b.readline()
                    if not left or not right:
                        raise ValueError('effective prompt sequence is incomplete')
                    x, y = json.loads(left), json.loads(right)
                    for row in (x, y):
                        if (row.get('seq') != index or row.get('cohort') != index // concurrency
                                or not re.fullmatch(r'[0-9a-f]{16}', str(row.get('prompt_sha256_16', '')))
                                or not isinstance(row.get('prompt_bytes'), int) or row['prompt_bytes'] < 1):
                            raise ValueError('invalid prompt sequence slot')
                    if (x['prompt_sha256_16'], x['prompt_bytes']) != (y['prompt_sha256_16'], y['prompt_bytes']):
                        raise ValueError('baseline and candidate effective prompt sequences differ')
            checks.append({'workload': workload, 'concurrency': concurrency, 'verified_slots': max(maxima) + 1,
                           'baseline_max_slot': maxima[0], 'candidate_max_slot': maxima[1]})
    return {'points': checks, 'matched_points': len(checks)}


def verify_candidate_headlines(points):
    """Require the new informational numbers without using them as gates."""
    if len(points) != 6:
        raise ValueError('candidate headline grid is incomplete')
    for point in points:
        headline = (point.get('candidate') or {}).get('headline') or {}
        first_any = headline.get('ttft_first_any_s')
        values = [headline.get('window_seconds'), headline.get('completion_tokens_per_second'),
                  first_any.get('p50') if isinstance(first_any, dict) else None]
        if (any(not isinstance(value, (int, float)) or isinstance(value, bool) or not math.isfinite(value) or value < 0
                for value in values) or values[0] <= 0 or not isinstance(first_any.get('n'), int) or first_any['n'] < 1):
            raise ValueError('candidate native window, throughput or first-any headline missing')
        counts = [headline.get(key) for key in ('attempted', 'successes', 'errors', 'usage_missing_count',
                                                'completion_tokens_total', 'prompt_tokens_total')]
        if (any(not isinstance(v, int) or isinstance(v, bool) or v < 0 for v in counts)
                or counts[0] == 0 or counts[0] != counts[1] + counts[2]
                or counts[3] != 0 or counts[4] == 0 or counts[5] == 0
                or not isinstance(headline.get('ttft_coverage'), dict)
                or headline['ttft_coverage'].get('measured') != counts[0]):
            raise ValueError('candidate headline numerator or coverage missing or inconsistent')
