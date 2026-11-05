# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Post-run BC-COMP-001 barrier over both retained 27B model bundles.

No GPU or server is started. Raw evidence remains private; this tool cannot
substitute for human provenance review of the two underlying runs.
"""
import argparse
import datetime as dt
import json
from pathlib import Path
import shutil
import sys

import manifest_tool
import perf_evidence_checks
import perf_legacy_oracle
import result_writer


def compare_bundle(root, case, lane, model_tag, manifest, manifest_hash, candidate_sha, recorded_root=None):
    root = Path(root).resolve()
    record_path = root / 'results' / (case + '.json')
    record = json.loads(record_path.read_text(encoding='utf-8'), object_pairs_hook=manifest_tool.unique_object)
    cleanup = json.loads((root / 'evidence/cleanup-verified.json').read_text(encoding='utf-8'))
    if (root / 'results/lane-error.json').exists() or cleanup != {'cleanup': 'verified'} or 'evidence/cleanup-verified.json' not in record.get('evidence_paths', []):
        raise ValueError(f'{case} cleanup not verified')
    if record.get('ID') != case or record.get('result') != 'PASS' or record.get('candidate_sha') != candidate_sha:
        raise ValueError(f'{case} is not a passing case for this candidate')
    versions = record.get('versions') or {}; config = manifest['lanes'][lane]
    if (versions.get('manifest_sha256') != manifest_hash or versions.get('candidate_binary_sha256') != manifest['candidate_binary_sha256']
            or versions.get('baseline_binary_sha256') != manifest['baseline_binary_sha256'] or versions.get('baseline_release') != 'v1.7.2'
            or versions.get('model') != config['model'] or versions.get('model_revision') != config['revision']
            or versions.get('server_image') != config['image']):
        raise ValueError(f'{case} frozen identity mismatch')
    perf = root / 'evidence/perf'
    models = json.loads((perf / 'models.json').read_text(encoding='utf-8'))
    if config['model'] not in {item.get('id') for item in models.get('data', []) if isinstance(item, dict)}:
        raise ValueError('server model identity missing')
    if lane == 'P2' and 'FlashInferCutlassNvFp4LinearKernel' not in (perf / 'vllm.log').read_text(encoding='utf-8', errors='replace'):
        raise ValueError('P2 expected kernel evidence missing')
    device = [part.strip() for part in (root / 'evidence/gpu-identity.csv').read_text(encoding='utf-8').strip().split(',')]
    gpu_csv = [part.strip() for part in (root / 'evidence/gpu.csv').read_text(encoding='utf-8').strip().split(',')]
    if (len(device) != 3 or device[0] != 'NVIDIA RTX PRO 6000 Blackwell Server Edition'
            or not device[1].startswith('GPU-') or int(device[2]) < 90000 or len(gpu_csv) != 2
            or gpu_csv != [device[0], record.get('driver')] or record.get('gpu_class') != device[0]):
        raise ValueError('specified GPU identity missing or inconsistent')
    prompts = {w: config['prompt_sets'][w]['sha256'] for w in ('W1', 'W2')}
    provenance = perf_evidence_checks.verify(perf, config['model'], manifest['candidate_binary_sha256'],
                                              manifest['baseline_binary_sha256'], prompts, recorded_root=recorded_root,
                                              prompt_paths={w: config['prompt_sets'][w]['path'] for w in ('W1', 'W2')})
    sequences = perf_evidence_checks.verify_sequences(perf)
    report = json.loads((perf / 'comparison.json').read_text(encoding='utf-8'))
    if report.get('model') != model_tag or report.get('pass') != '1' or report.get('tolerance_pct') != 5:
        raise ValueError('comparison model, pass or base threshold mismatch')
    perf_evidence_checks.verify_candidate_headlines(report['points'])
    noise = config.get('noise_floor'); floor = {}
    if noise:
        retained = root / 'evidence' / ('p1-noise-floor.json' if lane == 'P1' else 'p2-noise-floor.json')
        if (manifest_tool.sha256_file(retained) != noise['sha256'] or versions.get('noise_floor_sha256') != noise['sha256']
                or not str(noise.get('recorded_utc', '')).strip() or not str(noise.get('evidence_link', '')).strip()):
            raise ValueError('pre-recorded noise floor evidence mismatch')
        recorded = dt.datetime.fromisoformat(noise['recorded_utc'].replace('Z', '+00:00'))
        start = dt.datetime.fromisoformat(record['start_utc'].replace('Z', '+00:00'))
        if recorded.tzinfo is None or start.tzinfo is None or recorded >= start:
            raise ValueError('noise floor was not recorded before the run')
        floor = json.loads(retained.read_text(encoding='utf-8'))
    independent = perf_legacy_oracle.verify_grid(perf, report, model_tag, floor)
    if report.get('verdict') != 'PASS' or independent['result'] != 'PASS' or sequences['matched_points'] != 6:
        raise ValueError(f'{case} comparable metrics did not pass all six points')
    return {'case': case, 'versions': versions, 'gpu_class': record.get('gpu_class'), 'driver': record.get('driver'),
            'prompt_sha256': prompts, 'provenance': provenance, 'prompt_sequences': sequences,
            'independent_legacy': independent}


def copy_evidence(source, destination, case, lane):
    source = source.resolve(); destination.mkdir(parents=True)
    wanted = [source / 'results' / f'{case}.json']
    # Explicit paths rather than a blind recursive copy: retain the exact data
    # used by the barrier, not unrelated generated content or credentials.
    perf = source / 'evidence/perf'
    wanted += [perf / 'comparison.json', perf / 'comparison.md', perf / 'models.json', perf / 'vllm.log',
               source / 'evidence/gpu.csv', source / 'evidence/gpu-identity.csv', source / 'evidence/cleanup-verified.json']
    for label in ('bench-rel', 'bench-src'):
        for workload in ('W1', 'W2'):
            for concurrency in (1, 8, 32):
                point = perf / label / '1' / workload / f'c{concurrency}'
                for subdir in (point, point / 'warmup'):
                    for name in ('point.json', 'command.json', 'DONE'):
                        wanted.append(subdir / name)
                wanted += [point / 'requests.jsonl', point / 'prompt-sequence.jsonl']
    item = source / 'evidence' / f'{lane}-noise-floor.json'
    if item.exists(): wanted.append(item)
    case_record = json.loads((source / 'results' / f'{case}.json').read_text(encoding='utf-8'))
    for relative in case_record.get('evidence_paths', []):
        path = Path(relative)
        if path.is_absolute() or '..' in path.parts:
            raise ValueError('unsafe case evidence path')
        wanted.append(source / path)
    copied = []
    for path in dict.fromkeys(wanted):
        if path.is_symlink() or not path.resolve().is_relative_to(source) or not path.is_file():
            raise ValueError('missing, unsafe or external source evidence')
        target = destination / path.relative_to(source); target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(path, target)
        if manifest_tool.sha256_file(path) != manifest_tool.sha256_file(target):
            raise ValueError('source evidence changed while copying')
        copied.append(target)
    return copied


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('p1-out', 'p2-out', 'manifest', 'out'):
        parser.add_argument('--' + name, type=Path, required=True)
    for name in ('owner', 'evidence-link'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--attempt', type=int, default=1)
    args = parser.parse_args()
    if args.attempt < 1 or not args.owner.strip() or not args.evidence_link.strip():
        parser.error('positive attempt, owner and evidence link required')
    start = dt.datetime.now(dt.timezone.utc).isoformat().replace('+00:00', 'Z')
    args.out.mkdir(); (args.out / 'evidence').mkdir(); (args.out / 'results').mkdir()
    evidence = args.out / 'evidence'; observed = {}; passed = False
    sha = ''
    try:
        manifest = json.loads(args.manifest.read_text(encoding='utf-8'), object_pairs_hook=manifest_tool.unique_object)
        sha = manifest['candidate_sha']; manifest_hash = manifest_tool.sha256_file(args.manifest)
        left = compare_bundle(args.p1_out, 'BC-PERF-001', 'P1', 'q38-27b', manifest, manifest_hash, sha)
        right = compare_bundle(args.p2_out, 'BC-PERF-002', 'P2', 'q38-27b-nvfp4', manifest, manifest_hash, sha)
        if (left['prompt_sha256'] != right['prompt_sha256'] or left['gpu_class'] != right['gpu_class']
                or not left['gpu_class'] or not left['driver'] or not right['driver']):
            raise ValueError('two-model inputs or device-class provenance differ')
        # Copy under fixed aliases; evidence selection is independent of user directory names.
        for root, name, case in ((args.p1_out, 'p1', 'BC-PERF-001'), (args.p2_out, 'p2', 'BC-PERF-002')):
            copy_evidence(root, evidence / name, case, name)
        copied_left = compare_bundle(evidence / 'p1', 'BC-PERF-001', 'P1', 'q38-27b', manifest, manifest_hash, sha,
                                     recorded_root=args.p1_out.resolve() / 'evidence/perf')
        copied_right = compare_bundle(evidence / 'p2', 'BC-PERF-002', 'P2', 'q38-27b-nvfp4', manifest, manifest_hash, sha,
                                      recorded_root=args.p2_out.resolve() / 'evidence/perf')
        if copied_left != left or copied_right != right:
            raise ValueError('copied evidence differs from the validated source bundles')
        shutil.copyfile(args.manifest, evidence / 'manifest.json')
        if manifest_tool.sha256_file(evidence / 'manifest.json') != manifest_hash:
            raise ValueError('manifest changed while copying')
        observed = {'result': 'PASS', 'models': [left, right], 'comparable_points': 12,
                    'semantic_mapping': 'legacy success-only numerator and first measured send to last successful terminal; candidate native headline reported separately'}
        passed = True
    except (OSError, ValueError, TypeError, KeyError, json.JSONDecodeError) as exc:
        observed = {'result': 'FAIL', 'error': str(exc)}
    (evidence / 'comp001-observed.json').write_text(json.dumps(observed, indent=2, allow_nan=False) + '\n', encoding='utf-8')
    if not sha:
        print('Cannot write case without a valid candidate SHA; evidence retained.', file=sys.stderr)
        return 1
    files = [str(path.relative_to(args.out)) for path in evidence.rglob('*') if path.is_file()]
    versions = {'manifest_sha256': manifest_tool.sha256_file(args.manifest)}
    if passed: versions['models'] = [left['versions'], right['versions']]
    record = {'ID': 'BC-COMP-001', 'result': 'PASS' if passed else 'FAIL', 'candidate_sha': sha,
              'attempt': args.attempt, 'start_utc': start, 'end_utc': dt.datetime.now(dt.timezone.utc).isoformat().replace('+00:00', 'Z'),
              'owner': args.owner, 'evidence_link': args.evidence_link, 'gpu_class': left['gpu_class'] if passed else None,
              'driver': left['driver'] if passed else None, 'versions': versions,
              'observed_values': observed, 'thresholds': {'points': 12, 'tolerance': '5% or previously recorded wider point floor'},
              'evidence_paths': files}
    result_writer.write_record(args.out, record)
    return 0 if passed else 1


if __name__ == '__main__':
    sys.exit(main())
