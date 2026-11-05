# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Run the prospective 27B NVFP4 certification grid, never a CPU fixture."""
import argparse
import datetime as dt
import json
import os
from pathlib import Path
import signal
import shutil
import subprocess
import sys

import l1_runner
import manifest_tool
import perf_evidence_checks
import perf_baseline
import result_writer

ROOT = Path(__file__).resolve().parents[3]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ('out', 'candidate', 'baseline', 'manifest'):
        parser.add_argument('--' + key, type=Path, required=True)
    for key in ('sha', 'model', 'revision', 'image', 'start-utc', 'owner', 'evidence-link'):
        parser.add_argument('--' + key, required=True)
    args = parser.parse_args()
    evidence = args.out / 'evidence'
    gpu = (evidence / 'gpu.csv').read_text(encoding='utf-8').strip().split(',', 1)
    versions = {'server_image': args.image, 'model': args.model, 'model_revision': args.revision,
                'candidate_binary_sha256': manifest_tool.sha256_file(args.candidate),
                'baseline_binary_sha256': manifest_tool.sha256_file(args.baseline),
                'manifest_sha256': manifest_tool.sha256_file(args.manifest), 'baseline_release': 'v1.7.2'}
    observed, passed = {}, False
    try:
        manifest, config = manifest_tool.load_manifest(args.manifest, 'P2', args.candidate, args.baseline,
                                                       os.environ['CUDA_VISIBLE_DEVICES'])
        if manifest['candidate_sha'] != args.sha or config['model'] != args.model or config['revision'] != args.revision or config['image'] != args.image:
            raise ValueError('manifest identity differs from invocation')
        noise = config.get('noise_floor')
        noise_copy = None
        if noise:
            recorded = dt.datetime.fromisoformat(noise['recorded_utc'].replace('Z', '+00:00'))
            start = dt.datetime.fromisoformat(args.start_utc.replace('Z', '+00:00'))
            if recorded.tzinfo is None or start.tzinfo is None or recorded >= start or not str(noise.get('evidence_link', '')).strip():
                raise ValueError('noise floor must have prior timestamp and independently reviewable evidence')
            noise_copy = evidence / 'p2-noise-floor.json'
            shutil.copyfile(noise['path'], noise_copy)
            if manifest_tool.sha256_file(noise_copy) != noise['sha256']:
                raise ValueError('noise floor changed before collection')
            versions['noise_floor_sha256'] = noise['sha256']
            observed['noise_floor_provenance'] = {'recorded_utc': noise['recorded_utc'], 'evidence_link': noise['evidence_link']}
        hardware = subprocess.run(['nvidia-smi', f"--id={os.environ['CUDA_VISIBLE_DEVICES']}",
                    '--query-gpu=name,uuid,memory.total', '--format=csv,noheader,nounits'], check=True, capture_output=True, text=True, timeout=15)
        (evidence / 'gpu-identity.csv').write_text(hardware.stdout, encoding='utf-8')
        parts = [part.strip() for part in hardware.stdout.strip().split(',')]
        if (len(parts) != 3 or parts[0] != 'NVIDIA RTX PRO 6000 Blackwell Server Edition'
                or not parts[1].startswith('GPU-') or int(parts[2]) < 90000):
            raise ValueError('specified 96GB RTX PRO 6000 Blackwell Server Edition GPU required')
        dataset = Path(config['prompt_sets']['W1']['path']).resolve().parent
        target = evidence / 'perf'
        env = os.environ.copy()
        env.pop('TELEMETRY_YAML', None)
        env.update({'DATASETS_DIR': str(dataset), 'VLLM_IMAGE': args.image, 'PERF_OFFLINE': '1', 'WARMUP': '30', 'DURATION': '180'})
        command = ['bash', str(ROOT / 'scripts/validation/perf/runner.sh'), 'q38-27b-nvfp4',
                   str(args.baseline), str(args.candidate), str(target), '1']
        code = l1_runner.call(command, evidence / 'p2-runner-stdout.txt', evidence / 'p2-runner-stderr.txt', 14400, env=env)
        if code != 0:
            raise ValueError(f'performance runner failed or cleanup incomplete (exit {code})')
        models = json.loads((target / 'models.json').read_text(encoding='utf-8'))
        if args.model not in {item.get('id') for item in models.get('data', []) if isinstance(item, dict)}:
            raise ValueError('ready server model identity differs from the pinned model')
        if 'FlashInferCutlassNvFp4LinearKernel' not in (target / 'vllm.log').read_text(encoding='utf-8', errors='replace'):
            raise ValueError('expected NVFP4 kernel missing from server startup log')
        observed['provenance'] = perf_evidence_checks.verify(target, args.model, versions['candidate_binary_sha256'],
                                  versions['baseline_binary_sha256'], {w: config['prompt_sets'][w]['sha256'] for w in ('W1', 'W2')})
        comparison = target / 'comparison.json'
        compare_command = [sys.executable, str(ROOT / 'scripts/validation/perf/compare.py'),
                 '--baseline-root', str(target / 'bench-rel'), '--candidate-root', str(target / 'bench-src'),
                 '--model', 'q38-27b-nvfp4', '--pass', '1', '--tolerance-pct', '5',
                 '--out-md', str(target / 'comparison.md'), '--out-json', str(comparison)]
        if noise_copy:
            if manifest_tool.sha256_file(noise_copy) != noise['sha256']:
                raise ValueError('retained noise floor changed during collection')
            compare_command += ['--noise-floor', str(noise_copy)]
        code = l1_runner.call(compare_command, evidence / 'p2-compare-stdout.txt', evidence / 'p2-compare-stderr.txt', 180)
        if not comparison.is_file():
            raise ValueError(f'comparison failed without a complete report (exit {code})')
        value = json.loads(comparison.read_text(encoding='utf-8'))
        if manifest_tool.sha256_file(args.manifest) != versions['manifest_sha256']:
            raise ValueError('manifest changed during performance collection')
        perf_baseline.verify(config['baseline_asset']['path'], args.baseline)
        perf_evidence_checks.verify_candidate_headlines(value['points'])
        observed['comparison'] = value
        passed = code == 0 and value.get('verdict') == 'PASS'
        if not passed:
            observed['error'] = f'legacy-cohort performance gates did not pass (exit {code})'
    except (OSError, ValueError, TypeError, KeyError, subprocess.SubprocessError) as exc:
        observed['error'] = str(exc)
    output = evidence / 'p2-observed.json'
    output.write_text(json.dumps(observed, indent=2, allow_nan=False) + '\n', encoding='utf-8')
    files = [str(path.relative_to(args.out)) for path in evidence.rglob('*') if path.is_file()]
    record = {'ID': 'BC-PERF-002', 'result': 'PASS' if passed else 'FAIL', 'candidate_sha': args.sha,
              'attempt': int(os.environ['CERT_ATTEMPT']), 'start_utc': args.start_utc,
              'end_utc': dt.datetime.now(dt.timezone.utc).isoformat().replace('+00:00', 'Z'),
              'owner': args.owner, 'evidence_link': args.evidence_link, 'gpu_class': gpu[0].strip(),
              'driver': gpu[1].strip() if len(gpu) > 1 else '', 'versions': versions,
              'observed_values': observed, 'thresholds': {'points': 6, 'warmup_seconds': 30,
                  'measurement_seconds': 180, 'legacy_throughput_and_tpot_tolerance_pct': '5 or pre-recorded wider point floor',
                  'expected_kernel': 'FlashInferCutlassNvFp4LinearKernel'}, 'evidence_paths': files}
    result_writer.write_record(args.out, record)
    return 0 if passed else 1


if __name__ == '__main__':
    def interrupted(signum, _frame): raise SystemExit(128 + signum)
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    sys.exit(main())
