#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Exercise timeouts, real restarts and malformed real frames in both orders."""
import argparse
import datetime as dt
import json
import subprocess
from pathlib import Path

import g2_fault_checks
import g2_mixed_checks
import g3_runner
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
    for fault in ('timeout', 'corrupt'):
        for target in (1, 2):
            label = f'g2-fault-{fault}-{target}'
            try:
                kw = {'fault': 'corrupt', 'fault_request': target} if fault == 'corrupt' else {'delay_request': target, 'delay_seconds': 4.5, 'request_timeout': 2}
                data, trace, code, _ = r_runner.run_cell(args, label, args.candidate, 2, 128, False, ['--fail-on-error'], concurrency=1, **kw)
                cells[label] = g2_fault_checks.verify(data, trace, fault, target, code, timeout_s=2)
            except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, subprocess.SubprocessError) as exc:
                cells[label] = {'error': str(exc)}
                errors.append(label)
    for order in ('error-first', 'error-last'):
        key = f'g2-fault-restart-{order}'
        try:
            data, trace, code = g3_runner.run_cell(args, order, label='g2-fault-restart', extra_flags=['--fail-on-error'], concurrency=1)
            result = g2_mixed_checks.verify(data, trace, order)
            rows = [json.loads(line) for line in data.splitlines() if line.strip()]
            if code == 0 or not any((r.get('error') or {}).get('kind') == 'stream_truncated' for r in rows if '.request.' in r.get('schema_version', '')):
                raise ValueError('safety exit or real truncated-stream classification missing')
            cells[key] = {**result, 'exit_code': code}
        except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, subprocess.SubprocessError) as exc:
            cells[key] = {'error': str(exc)}
            errors.append(key)
    files = ['evidence/' + name for name in ('gpu.csv', 'server.log', 'binary-hashes.txt', 'container-id.txt')]
    files += [str(path.relative_to(args.out)) for path in evidence.glob('g2-fault-*') if path.is_file()]
    record = {'ID': 'BC-GPU-007', 'result': 'FAIL' if errors else 'PASS', 'candidate_sha': args.sha, 'attempt': 1,
              'start_utc': args.start_utc, 'end_utc': dt.datetime.now(dt.timezone.utc).isoformat().replace('+00:00', 'Z'),
              'owner': args.owner, 'evidence_link': args.evidence_link, 'gpu_class': gpu[0].strip(),
              'driver': gpu[1].strip() if len(gpu) > 1 else '',
              'versions': {'server_image': args.image, 'model': args.model, 'model_revision': args.revision,
                           'candidate_binary_sha256': manifest_tool.sha256_file(args.candidate),
                           'baseline_binary_sha256': manifest_tool.sha256_file(args.baseline),
                           'manifest_sha256': manifest_tool.sha256_file(args.manifest)},
              'observed_values': {'cells': cells, 'failed_cells': errors},
              'thresholds': {'orders': 2, 'faults': ['timeout', 'corrupt', 'restart'], 'timeout_s': 2,
                             'timeout_overrun_limit_s': 5, 'safety_exit_nonzero': True}, 'evidence_paths': files}
    result_writer.write_record(args.out, record)
    return 1 if errors else 0


if __name__ == '__main__':
    raise SystemExit(main())
