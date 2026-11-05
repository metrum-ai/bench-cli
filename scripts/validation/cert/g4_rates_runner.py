#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Check every-rate mixed orderings plus isolated real server counters."""
import argparse
import datetime as dt
import json
import os
import subprocess
from pathlib import Path

import g3_checks
import g3_runner
import g4_rates_checks
import g6_checks
import g6_counter_runner
import manifest_tool
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
    try:
        before = g6_counter_runner.snapshot(args.port, evidence / 'g4-counter-before.txt')
        data, trace, code, _ = r_runner.run_cell(args, 'g4-counter', args.candidate, 64, 128, True, [], concurrency=16)
        after = g6_counter_runner.snapshot(args.port, evidence / 'g4-counter-after.txt')
        if code != 0:
            raise ValueError('isolated real counter control exited nonzero')
        inventory = json.loads((evidence / 'models.json').read_text(encoding='utf-8'))
        label, kind = g6_checks.metrics_model_label(before, after, args.model, args.revision, inventory, bool(os.environ.get('CERT_HF_HUB_CACHE')))
        cells['counter-control'] = g6_checks.evaluate(data, trace, before, after, label)
        cells['counter-control']['counter_model_label_kind'] = kind
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, subprocess.SubprocessError) as exc:
        cells['counter-control'] = {'error': str(exc)}; errors.append('counter-control')
    for order in ('control', 'error-first', 'error-last'):
        try:
            data, trace, code = g3_runner.run_cell(args, order, label='g4-rates')
            window_check = g3_checks.verify_complete_control(data, trace) if order == 'control' else g3_checks.verify_window(data, trace, order)
            cells[order] = {'rate_checks': g4_rates_checks.verify(data, trace, g3_runner.E2E_SLO_SECONDS),
                            'window_checks': window_check, 'exit_code': code}
        except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, subprocess.SubprocessError) as exc:
            cells[order] = {'error': str(exc)}; errors.append(order)
    if not errors and not any(cells[order]['rate_checks']['failure_answer_overlaps'] for order in ('error-first', 'error-last')):
        cells['overlap'] = {'error': 'no real failure overlapped a successful answer'}; errors.append('overlap')
    files = ['evidence/' + name for name in ('gpu.csv', 'server.log', 'binary-hashes.txt', 'container-id.txt', 'models.json')]
    files += [str(path.relative_to(args.out)) for path in evidence.glob('g4-*') if path.is_file()]
    record = {'ID': 'BC-GPU-012', 'result': 'FAIL' if errors else 'PASS', 'candidate_sha': args.sha, 'attempt': 1,
              'start_utc': args.start_utc, 'end_utc': dt.datetime.now(dt.timezone.utc).isoformat().replace('+00:00', 'Z'),
              'owner': args.owner, 'evidence_link': args.evidence_link, 'gpu_class': gpu[0].strip(),
              'driver': gpu[1].strip() if len(gpu) > 1 else '',
              'versions': {'server_image': args.image, 'model': args.model, 'model_revision': args.revision,
                           'candidate_binary_sha256': manifest_tool.sha256_file(args.candidate),
                           'baseline_binary_sha256': manifest_tool.sha256_file(args.baseline),
                           'manifest_sha256': manifest_tool.sha256_file(args.manifest)},
              'observed_values': {'cells': cells, 'failed_cells': errors},
              'thresholds': {'rate_tolerance_pct': 1, 'counter_tolerance_pct': 5, 'counter_absolute_tolerance_tokens': 1,
                             'window_tolerance_s': .05, 'e2e_slo_s': g3_runner.E2E_SLO_SECONDS}, 'evidence_paths': files}
    result_writer.write_record(args.out, record)
    return 1 if errors else 0


if __name__ == '__main__':
    raise SystemExit(main())
