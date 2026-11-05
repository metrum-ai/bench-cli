#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Candidate xtask local smoke and independent headline for L2/L3/L4."""
import argparse
import datetime as dt
import json
import os
from pathlib import Path

import l1_runner
import live_modality_checks
import manifest_tool
import result_writer

ROOT = Path(__file__).resolve().parents[3]
CASES = {'vlm': ('BC-LIVE-002', 8), 'asr': ('BC-LIVE-003', 6), 'imagegen': ('BC-LIVE-004', 4)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--modality', choices=CASES, required=True)
    for name in ('out', 'candidate', 'baseline', 'manifest', 'prompt', 'sut'):
        parser.add_argument('--' + name, type=Path, required=True)
    for name in ('sha', 'model', 'revision', 'image', 'start-utc', 'owner', 'evidence-link'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--port', type=int, required=True)
    args = parser.parse_args()
    evidence = args.out / 'evidence'
    case, count = CASES[args.modality]
    directory = evidence / ('live-' + args.modality)
    data = directory / 'results.jsonl'
    artifact_dir = directory / 'artifacts'
    stdout, stderr = evidence / 'live-smoke-stdout.txt', evidence / 'live-smoke-stderr.txt'
    headline_out, headline_err = evidence / 'live-headline-stdout.txt', evidence / 'live-headline-stderr.txt'
    env = os.environ.copy()
    env.update({'CARGO_NET_OFFLINE': 'true', 'BENCH_BIN_DIR': str(args.candidate.parent), 'MIN_SUCCESS_RATIO': '1.0'})
    try:
        config = json.loads(args.manifest.read_text(encoding='utf-8'))['lanes'][{'vlm': 'L2', 'asr': 'L3', 'imagegen': 'L4'}[args.modality]]
        modality_binary = args.candidate.parent / ('metrum-ai-bench-cli-' + args.modality)
        if manifest_tool.sha256_file(modality_binary) != config['modality_binary']['sha256']:
            raise ValueError('candidate modality binary changed after preflight')
        if args.modality == 'imagegen':
            prompt = args.prompt.read_text(encoding='utf-8').strip()
            if not prompt or len(prompt) > 4096 or '\x00' in prompt:
                raise ValueError('invalid pinned image-generation prompt')
            env['IMAGEGEN_PROMPT'] = prompt
        command = ['cargo', '--locked', 'xtask', 'local-smoke', args.modality, '--url', f'http://localhost:{args.port}',
                   '--model', args.model, '--sut', str(args.sut), '--out', str(directory), '--num-requests', str(count),
                   '--concurrency', '2', '--warmup', '0', '--seed', '42', '--root', str(ROOT)]
        headline = ['cargo', '--locked', 'xtask', 'assert-headline', args.modality, str(data)]
        if args.modality == 'imagegen': headline += ['--artifact-dir', str(artifact_dir)]
        (evidence / 'live-commands.json').write_text(json.dumps({'smoke': command, 'headline': headline,
            'prompt_sha256': manifest_tool.sha256_file(args.prompt), 'num_requests': count}, indent=2) + '\n', encoding='utf-8')
        smoke = l1_runner.call(command, stdout, stderr, 1800, env=env)
        assertion = l1_runner.call(headline, headline_out, headline_err, 300, env=env)
        artifacts = [path for path in artifact_dir.rglob('*') if path.is_file() and path.suffix.lower() in ('.png', '.jpg', '.jpeg')] if artifact_dir.exists() else []
        observed = live_modality_checks.evaluate(data.read_text(encoding='utf-8'), args.modality, smoke, assertion, len(artifacts))
        if observed['attempted'] != count:
            raise ValueError('measured request count differs from frozen run')
        if manifest_tool.sha256_file(modality_binary) != config['modality_binary']['sha256']:
            raise ValueError('candidate modality binary changed during execution')
        models = json.loads((evidence / 'models.json').read_text(encoding='utf-8'))
        if not any(row.get('id') == args.model for row in models.get('data', [])):
            raise ValueError('server did not serve the pinned model')
        passed = True
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as exc:
        (evidence / 'live-evaluation-error.txt').write_text(f'{type(exc).__name__}: {exc}\n', encoding='utf-8')
        observed, passed = {'error': type(exc).__name__, 'detail': 'see private evaluation evidence'}, False
    gpu = (evidence / 'gpu.csv').read_text(encoding='utf-8').strip().split(',', 1)
    config = json.loads(args.manifest.read_text(encoding='utf-8'))['lanes'][{'vlm': 'L2', 'asr': 'L3', 'imagegen': 'L4'}[args.modality]]
    files = [str(path.relative_to(args.out)) for path in evidence.rglob('*') if path.is_file()]
    record = {'ID': case, 'result': 'PASS' if passed else 'FAIL', 'candidate_sha': args.sha, 'attempt': 1,
              'start_utc': args.start_utc, 'end_utc': dt.datetime.now(dt.timezone.utc).isoformat().replace('+00:00', 'Z'),
              'owner': args.owner, 'evidence_link': args.evidence_link, 'gpu_class': gpu[0].strip(),
              'driver': gpu[1].strip() if len(gpu) > 1 else '',
              'versions': {'server_image': args.image, 'model': args.model, 'model_revision': args.revision,
                           'candidate_binary_sha256': manifest_tool.sha256_file(args.candidate),
                           'baseline_binary_sha256': manifest_tool.sha256_file(args.baseline),
                           'modality_binary_sha256': config['modality_binary']['sha256'],
                           'manifest_sha256': manifest_tool.sha256_file(args.manifest)},
              'observed_values': observed, 'thresholds': {'measured': count, 'success_ratio': 1.0, 'required_sut': True,
                                                         'all_saved_images_decode': args.modality == 'imagegen'},
              'evidence_paths': files}
    result_writer.write_record(args.out, record)
    return 0 if passed else 1


if __name__ == '__main__':
    raise SystemExit(main())
