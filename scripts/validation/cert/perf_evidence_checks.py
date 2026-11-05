# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""CPU-side evidence checks for the unchanged 27B performance point grid.

These checks do not launch a server or issue a case result. A release-specific
baseline decision and both model bundles are still required by the lane.
"""
import json
import math
from pathlib import Path


def option(argv, name):
    positions = [i for i, value in enumerate(argv) if value == name]
    if len(positions) != 1 or positions[0] + 1 >= len(argv):
        raise ValueError(f'missing or duplicate {name}')
    return argv[positions[0] + 1]


def verify(root, model, candidate_hash, baseline_hash, prompt_hashes):
    root = Path(root)
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
                            or json.loads(option(argv, '--extra-body-json')) != {'seed': 42}):
                        raise ValueError('performance configuration differs from frozen spec')
                checked.append(f'{label}/{workload}/c{concurrency}')
    return {'verified_phases': len(checked) * 2, 'verified_tool_points': len(checked)}


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
