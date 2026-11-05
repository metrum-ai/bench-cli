#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""G2 real mixed outcomes and ordering across TTFT switches."""
import argparse
import datetime as dt
import json
import subprocess
from pathlib import Path

import g2_mixed_checks
import g3_runner
import manifest_tool
import r_checks
import r_runner
import result_writer


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('out', 'candidate', 'baseline', 'manifest', 'prompt'):
        parser.add_argument('--' + name, type=Path, required=True)
    for name in ('sha', 'model', 'revision', 'image', 'start-utc', 'owner', 'evidence-link'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--port', type=int, required=True)
    parser.add_argument('--proxy-port', type=int, required=True)
    args = parser.parse_args()
    evidence = args.out / 'evidence'
    gpu = (evidence / 'gpu.csv').read_text(encoding='utf-8').strip().split(',', 1)
    cells, errors = {}, []
    flags = [('default', [], False), ('infer-first-byte', ['--infer-ttft-from-first-byte'], False),
             ('first-token-alias', ['--ttft-fallback', 'first-token'], False),
             ('allow-missing', ['--allow-missing-ttft'], False),
             ('strict', ['--strict-reasoning-truncation'], True)]
    for name, extra, strict in flags:
        for order in ('error-first', 'error-last'):
            label = f'g2-mixed-{name}'
            key = f'{name}-{order}'
            try:
                data, trace, code = g3_runner.run_cell(args, order, label=label, extra_flags=extra, concurrency=1)
                observed = g2_mixed_checks.verify(data, trace, order, strict)
                if strict and code == 0:
                    raise ValueError('strict truncation must exit nonzero')
                cells[key] = {**observed, 'exit_code': code}
            except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, subprocess.SubprocessError) as exc:
                cells[key] = {'error': str(exc)}
                errors.append(key)
        if name == 'default' and all('error' in cells[f'default-{order}'] for order in ('error-first', 'error-last')):
            cells['remaining-flag-cells'] = {'error': 'not executed: both baseline interruption cells failed'}
            errors.append('remaining-flag-cells')
            break
    try:
        data, trace, code, _ = r_runner.run_cell(args, 'g2-mixed-thinking-off', args.candidate, 8, 512, False, [], concurrency=1)
        cells['thinking-off'] = r_checks.control_cell(data, trace, 8, code)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, subprocess.SubprocessError) as exc:
        cells['thinking-off'] = {'error': str(exc)}
        errors.append('thinking-off')
    files = ['evidence/' + name for name in ('gpu.csv', 'server.log', 'binary-hashes.txt', 'container-id.txt')]
    files += [str(path.relative_to(args.out)) for path in evidence.glob('g2-mixed-*') if path.is_file()]
    record = {'ID': 'BC-GPU-005', 'result': 'FAIL' if errors else 'PASS', 'candidate_sha': args.sha, 'attempt': 1,
              'start_utc': args.start_utc, 'end_utc': dt.datetime.now(dt.timezone.utc).isoformat().replace('+00:00', 'Z'),
              'owner': args.owner, 'evidence_link': args.evidence_link, 'gpu_class': gpu[0].strip(),
              'driver': gpu[1].strip() if len(gpu) > 1 else '',
              'versions': {'server_image': args.image, 'model': args.model, 'model_revision': args.revision,
                           'candidate_binary_sha256': manifest_tool.sha256_file(args.candidate),
                           'baseline_binary_sha256': manifest_tool.sha256_file(args.baseline),
                           'manifest_sha256': manifest_tool.sha256_file(args.manifest)},
              'observed_values': {'cells': cells, 'failed_cells': errors},
              'thresholds': {'requests_per_mixed_cell': 16, 'answers_min': 1, 'truncations_min': 1,
                             'real_interruption_failures_min': 1, 'flags_and_orders': 10}, 'evidence_paths': files}
    result_writer.write_record(args.out, record)
    return 1 if errors else 0


if __name__ == '__main__':
    raise SystemExit(main())
