# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Finalize W only after an external Metrum Bench platform-team sign-off."""
import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import sys

import manifest_tool
import result_writer
import w_checks


def finalize(root, signoff_path):
    root = Path(root).resolve(); evidence = root / 'evidence'
    collection_file = evidence / 'w-collection.json'; cleanup_file = evidence / 'cleanup-verified.json'
    collection_bytes = collection_file.read_bytes()
    collection = json.loads(collection_bytes, object_pairs_hook=manifest_tool.unique_object)
    signoff_bytes = Path(signoff_path).read_bytes()
    signoff = json.loads(signoff_bytes, object_pairs_hook=manifest_tool.unique_object)
    if ((root / 'results/lane-error.json').exists() or json.loads(cleanup_file.read_text(encoding='utf-8')) != {'cleanup': 'verified'}
            or not (evidence / 'server-stop.txt').is_file()):
        raise ValueError('W collection or owned-server cleanup failed')
    versions = collection['versions']; inputs = collection['input']
    if os.environ.get('CERT_ATTEMPT') not in (None, str(collection['attempt'])):
        raise ValueError('finalization attempt differs from collected run')
    required = {'team': 'Metrum Bench platform', 'approved': True, 'candidate_sha': collection['candidate_sha'],
                'collection_sha256': hashlib.sha256(collection_bytes).hexdigest(), 'cleanup_sha256': manifest_tool.sha256_file(cleanup_file),
                'server_log_sha256': manifest_tool.sha256_file(evidence / 'server.log'), 'wrapper_sha256': versions['wrapper_sha256']}
    if not isinstance(signoff, dict) or any(signoff.get(key) != value for key, value in required.items()):
        raise ValueError('platform sign-off identity or evidence hash mismatch')
    for key in ('signer', 'evidence_link'):
        if not isinstance(signoff.get(key), str) or not signoff[key].strip(): raise ValueError('platform sign-off lacks signer or evidence link')
    signed = dt.datetime.fromisoformat(signoff['signed_utc'].replace('Z', '+00:00'))
    collected = dt.datetime.fromisoformat(collection['collected_utc'].replace('Z', '+00:00'))
    if signed.tzinfo is None or collected.tzinfo is None or not collected <= signed <= dt.datetime.now(dt.timezone.utc):
        raise ValueError('platform sign-off timestamp is invalid')
    files = collection['files']
    required_files = {'evidence/gpu.csv', 'evidence/binary-hashes.txt', 'evidence/models.json'}
    for mode in ('adequate', 'strict', 'safety'):
        required_files.update(f'evidence/w-{mode}-{suffix}' for suffix in
                              ('cli.jsonl', 'wrapper.json', 'trace.jsonl', 'stdout.txt', 'stderr.txt', 'proxy-error.txt', 'command.json'))
    if not isinstance(files, dict) or not required_files <= files.keys(): raise ValueError('W evidence list incomplete')
    for name, digest in files.items():
        path = Path(name)
        if path.is_absolute() or '..' in path.parts or (root / path).is_symlink() or not (root / path).resolve().is_relative_to(root):
            raise ValueError('unsafe W evidence path')
        if manifest_tool.sha256_file(root / path) != digest: raise ValueError('W evidence changed after collection')
    for key, expected in (('candidate', versions['candidate_binary_sha256']), ('baseline', versions['baseline_binary_sha256']),
                          ('manifest', versions['manifest_sha256']), ('wrapper', versions['wrapper_sha256']), ('prompt', inputs['prompt_sha256'])):
        if manifest_tool.sha256_file(inputs[key]) != expected: raise ValueError('W frozen input changed')
    for mode in ('adequate', 'strict', 'safety'):
        cli = evidence / f'w-{mode}-cli.jsonl'; wrapper = evidence / f'w-{mode}-wrapper.json'; trace = evidence / f'w-{mode}-trace.jsonl'
        checked = w_checks.evaluate(cli.read_text(encoding='utf-8'), trace.read_text(encoding='utf-8'),
                   json.loads(wrapper.read_text(encoding='utf-8'), object_pairs_hook=manifest_tool.unique_object), inputs['count'], mode,
                   collection['observed'][mode]['wrapper_exit_code'], inputs['candidate'], inputs['url'], inputs['prompt'], cli,
                   versions['model'])
        expected = {k: v for k, v in collection['observed'][mode].items() if k != 'wrapper_exit_code'}
        if checked != expected: raise ValueError('W rechecked results differ')
    if manifest_tool.sha256_file(collection_file) != hashlib.sha256(collection_bytes).hexdigest():
        raise ValueError('W collection changed during review')
    signoff_copy = evidence / 'w-platform-signoff.json'
    with signoff_copy.open('xb') as out: out.write(signoff_bytes)
    record = {'ID': 'BC-INTEG-001', 'result': 'PASS', 'candidate_sha': collection['candidate_sha'], 'attempt': collection['attempt'],
              'start_utc': collection['start_utc'], 'end_utc': dt.datetime.now(dt.timezone.utc).isoformat().replace('+00:00', 'Z'),
              'owner': collection['owner'], 'evidence_link': signoff['evidence_link'], 'gpu_class': collection['gpu_class'],
              'driver': collection['driver'], 'versions': versions, 'observed_values': collection['observed'],
              'thresholds': {'answered_min': 1, 'truncated_min_per_strict_cell': 1, 'reasoning_streams_min': 1,
                             'wrapper_fields_and_counts_match': True, 'external_platform_signoff': 'manual review required'},
              'evidence_paths': sorted(set(files) | {'evidence/w-collection.json', 'evidence/cleanup-verified.json',
                                                    'evidence/server.log', 'evidence/server-stop.txt', 'evidence/w-platform-signoff.json'})}
    result_writer.write_record(root, record)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True); parser.add_argument('--signoff', type=Path, required=True)
    args = parser.parse_args()
    try:
        finalize(args.out, args.signoff)
        return 0
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as exc:
        print(f'W not certified: {exc}', file=sys.stderr)
        return 1


if __name__ == '__main__': sys.exit(main())
