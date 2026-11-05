#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Real streaming/nonstreaming and timeout GPU development case."""
import argparse
import datetime as dt
import json
import subprocess
from pathlib import Path

import g2_fault_checks
import g5_modes_checks
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
    for name, streaming, firstbyte in (('stream', True, False), ('nonstream', False, False), ('nonstream-firstbyte', False, True)):
        try:
            extra = ['--infer-ttft-from-first-byte'] if firstbyte else []
            data, trace, code, _ = r_runner.run_cell(args, f'g5-modes-{name}', args.candidate, 4, 128, False, extra,
                                                      concurrency=1, streaming=streaming, record_identity=True)
            if code != 0:
                raise ValueError(f'client exited {code}')
            cells[name] = g5_modes_checks.evaluate(data, trace, streaming, firstbyte, args.model)
        except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, subprocess.SubprocessError) as exc:
            cells[name] = {'error': str(exc)}; errors.append(name)
    if not errors and not (cells['stream']['prompt_sha256'] == cells['nonstream']['prompt_sha256'] == cells['nonstream-firstbyte']['prompt_sha256']):
        errors.append('prompt-identity')
        cells['prompt-identity'] = {'error': 'pinned prompt order differs across modes'}
    try:
        data, trace, code, _ = r_runner.run_cell(args, 'g5-modes-timeout', args.candidate, 2, 128, False,
                                                 ['--fail-on-error'], concurrency=1, delay_request=2,
                                                 delay_seconds=4.5, request_timeout=2)
        cells['timeout'] = g2_fault_checks.verify(data, trace, 'timeout', 2, code, timeout_s=2)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, subprocess.SubprocessError) as exc:
        cells['timeout'] = {'error': str(exc)}; errors.append('timeout')
    files = ['evidence/' + name for name in ('gpu.csv', 'server.log', 'binary-hashes.txt')]
    files += [str(path.relative_to(args.out)) for path in evidence.glob('g5-modes-*') if path.is_file()]
    record = {'ID': 'BC-GPU-014', 'result': 'FAIL' if errors else 'PASS', 'candidate_sha': args.sha, 'attempt': 1,
              'start_utc': args.start_utc, 'end_utc': dt.datetime.now(dt.timezone.utc).isoformat().replace('+00:00', 'Z'),
              'owner': args.owner, 'evidence_link': args.evidence_link, 'gpu_class': gpu[0].strip(),
              'driver': gpu[1].strip() if len(gpu) > 1 else '',
              'versions': {'server_image': args.image, 'model': args.model, 'model_revision': args.revision,
                           'candidate_binary_sha256': manifest_tool.sha256_file(args.candidate),
                           'baseline_binary_sha256': manifest_tool.sha256_file(args.baseline),
                           'manifest_sha256': manifest_tool.sha256_file(args.manifest)},
              'observed_values': {'cells': cells, 'failed_cells': errors},
              'thresholds': {'requests_per_mode': 4, 'answers_min': 1, 'timeout_s': 2, 'timeout_overrun_limit_s': 5},
              'evidence_paths': files}
    result_writer.write_record(args.out, record)
    return 1 if errors else 0


if __name__ == '__main__':
    raise SystemExit(main())
