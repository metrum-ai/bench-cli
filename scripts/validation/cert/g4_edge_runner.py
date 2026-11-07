#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Real GPU-010 edge cases; never produces GPU PASS without real evidence."""
import argparse
import datetime as dt
import json
import subprocess
from pathlib import Path

import g4_edge_checks
import gpu010_builds
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
        frozen_builds = gpu010_builds.from_environment(args)
    except (OSError, ValueError, KeyError, TypeError) as exc:
        frozen_builds = None
        cells['build-config'] = {'error': str(exc)}
        errors.append('build-config')
    try:
        config = json.loads(args.manifest.read_text(encoding='utf-8'))['lanes']['G4']
        tokenizer = config.get('tokenizer')
        if not isinstance(tokenizer, dict) or manifest_tool.sha256_file(Path(tokenizer['path'])) != tokenizer['sha256']:
            raise ValueError('pinned tokenizer.json and its hash are required')
    except (OSError, ValueError, KeyError, TypeError) as exc:
        tokenizer = None
        cells['tokenizer-config'] = {'error': str(exc)}
        errors.append('tokenizer-config')
    for name in gpu010_builds.CELLS:
        try:
            if frozen_builds is None:
                raise ValueError('frozen per-cell build provenance missing; no measured request allowed')
            binary, proof = gpu010_builds.cell(frozen_builds, name)
            # This immutable receipt exists before run_cell can issue a request.
            with (evidence / ('g4-edge-' + name + '-build.json')).open('x') as stream:
                json.dump(proof, stream, indent=2, sort_keys=True)
                stream.write('\n')
            if name == 'fallback' and tokenizer is None:
                raise ValueError('pinned tokenizer configuration missing')
            count = 4
            cap = 128
            thinking = name == 'truncated'
            fault = 'remove_usage' if name in ('missing', 'fallback') else None
            extra = ['--stop-after-seconds', '0'] if name == 'empty' else []
            if name == 'fallback':
                extra += ['--tokenizer', tokenizer['path']]
            data, trace, code, _ = r_runner.run_cell(args, 'g4-edge-' + name, binary, count, cap, thinking, extra,
                                                      concurrency=1, fault=fault)
            with (evidence / ('g4-edge-' + name + '-outcome.json')).open('x') as stream:
                json.dump({'exit_code': code}, stream, sort_keys=True)
                stream.write('\n')
            cells[name] = {**g4_edge_checks.check_cell(name, data, trace, code), 'build': proof}
        except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, subprocess.SubprocessError) as exc:
            cells[name] = {'error': str(exc)}
            errors.append(name)
    files = ['evidence/' + name for name in ('gpu.csv', 'server.log', 'binary-hashes.txt')]
    files += [str(path.relative_to(args.out)) for path in evidence.glob('g4-edge-*') if path.is_file()]
    record = {'ID': 'BC-GPU-010', 'result': 'FAIL' if errors else 'PASS', 'candidate_sha': args.sha, 'attempt': 1,
              'start_utc': args.start_utc, 'end_utc': dt.datetime.now(dt.timezone.utc).isoformat().replace('+00:00', 'Z'),
              'owner': args.owner, 'evidence_link': args.evidence_link, 'gpu_class': gpu[0].strip(),
              'driver': gpu[1].strip() if len(gpu) > 1 else '',
              'versions': {'server_image': args.image, 'model': args.model, 'model_revision': args.revision,
                           'candidate_binary_sha256': manifest_tool.sha256_file(args.candidate),
                           'baseline_binary_sha256': manifest_tool.sha256_file(args.baseline),
                           'manifest_sha256': manifest_tool.sha256_file(args.manifest),
                           'tokenizer_sha256': tokenizer['sha256'] if tokenizer else None,
                           'feature_variant_binary_sha256': frozen_builds[gpu010_builds.VARIANT_KEY]['sha256'] if frozen_builds else None,
                           'feature_variant_label': 'feature-variant build (tokenizer)' if frozen_builds else None},
              'observed_values': {'cells': cells, 'failed_cells': errors},
              'thresholds': {'requests_per_live_cell': 4, 'empty_issuance_count': 0, 'legacy_rate_tolerance_pct': 1},
              'evidence_paths': files}
    result_writer.write_record(args.out, record)
    return 1 if errors else 0


if __name__ == '__main__':
    raise SystemExit(main())
