# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""KVC adapter: offline data/export correctness, no serving stack or GPU."""
import argparse
import sys
from pathlib import Path

import build_roles
import execution_policy
import harness_integrity
import kv_cpu_checks as checks
import kv_cpu_runner as runner
import kv_envelope
import pretag_artifact
import validate_results
from execution_evidence import digest

AVAILABLE = True


def validate_spec(spec):
    required = {'schema_version', 'case', 'cases', 'candidate_sha', 'tools', 'public_revision', 'public_inputs',
                'inputs', 'workflow_origin', 'binary', 'probe', 'timeout_seconds', 'cleanup_grace_seconds',
                'cgroup_root', 'owner', 'evidence_link'}
    checks.require(isinstance(spec, dict) and set(spec) == required, 'unknown or missing CPU execution spec fields')
    checks.require(spec.get('schema_version') == 'kv-cpu-execution.v1', 'unsupported CPU execution spec')
    checks.require(spec.get('case') in runner.CASES and set(spec.get('cases', {})) == {spec['case']},
                   'one matching CPU case required')
    checks.require(validate_results.SHA.fullmatch(spec.get('candidate_sha', '')), 'full CPU candidate SHA required')
    tools = execution_policy.validate_case_tools(spec['tools'], 'KVC')
    checks.require('dirname' in tools, 'CPU shell entry requires pinned dirname')
    checks.require(tools['python3']['path'] == str(Path(sys.executable).resolve()), 'CPU interpreter pin mismatch')
    lock = checks.fixture_lock()
    checks.require(spec['public_revision'] == lock['public_inputs']['revision'], 'CPU public revision mismatch')
    public = Path(spec['public_inputs'])
    checks.require(public.is_absolute() and public.resolve() == public, 'canonical CPU public input directory required')
    if spec['case'] in ('BC-KV-010', 'BC-KV-011', 'BC-KV-013', 'BC-KV-014'):
        checks.verify_files(public, lock['public_inputs']['files'])
    declared = {str(kv_envelope.pin(item)) for item in kv_envelope.pins(spec.get('inputs', []))}
    expected = {str(checks.FIXTURES / name) for name in lock['files']} | {str(checks.FIXTURES / 'lock.json')}
    expected |= {str(runner.REPO / name) for name in lock['schemas']}
    if spec['case'] in ('BC-KV-010', 'BC-KV-011', 'BC-KV-013', 'BC-KV-014'):
        expected |= {str(public / name) for name in lock['public_inputs']['files']}
    checks.require(expected <= declared, 'CPU fixture/public/schema input closure incomplete')
    members = pretag_artifact.verify(kv_envelope.load(kv_envelope.pin(spec['workflow_origin'])), spec['candidate_sha'])
    names = {'binary': 'metrum-ai-bench-cli-prompts', 'probe': 'metrum-ai-bench-cli-cert-kv-cpu'}
    for key in ('binary', 'probe'):
        binary = kv_envelope.pin(spec[key], executable=True)
        checks.require(binary.name == names[key] and members.get(binary.name) == digest(binary),
                       'CPU executable is not in measured workflow archive')
    checks.require(type(spec['timeout_seconds']) is int and type(spec['cleanup_grace_seconds']) is int and
                   0 < spec['timeout_seconds'] <= 900 and 0 < spec['cleanup_grace_seconds'] < spec['timeout_seconds'],
                   'CPU time envelope must be positive and at most 900 seconds')
    for key in ('owner', 'evidence_link'):
        checks.require(isinstance(spec.get(key), str) and spec[key].strip(), 'CPU execution provenance required')
    return spec


def preflight(manifest):
    cases = kv_envelope.validate(manifest, lane='KVC')
    spec = validate_spec(kv_envelope.load(kv_envelope.pin(manifest['lane_manifest'])))
    checks.require(spec['case'] == next(iter(cases)), 'CPU case differs from frozen envelope')
    checks.require(manifest['candidate']['path'] == spec['binary']['path'] and
                   manifest['candidate']['sha256'] == spec['binary']['sha256'], 'CPU primary artifact differs from envelope')
    return cases


def freeze(args):
    spec = validate_spec(kv_envelope.load(args.lane_manifest))
    checks.require(spec['case'] == args.case, 'CPU freeze case mismatch')
    rows = validate_results.load_catalog(args.catalog)
    selection = validate_results.load_selection(args.selection, args.catalog, rows, spec['candidate_sha'])
    checks.require(args.case in selection['selected'] and rows[args.case]['Parallel group'] == 'KVC', 'CPU case not selected')
    checks.require(str(args.candidate.resolve()) == spec['binary']['path'], 'CPU measured candidate path mismatch')
    measured, _ = build_roles.load_receipt(args.build_receipt, spec['candidate_sha'])
    matching = [item for item in measured if item['path'] == spec['binary']['path'] and item['sha256'] == spec['binary']['sha256']]
    checks.require(len(matching) == 1, 'CPU primary is not a measured build artifact')
    checks.require(execution_policy.load_manifest(args.tool_manifest) == spec['tools'], 'CPU tool manifest mismatch')
    root = runner.REPO
    closure = harness_integrity.verify(root, spec['candidate_sha'])
    def pin(path):
        path = Path(path).resolve(strict=True)
        return {'path': str(path), 'sha256': digest(path)}
    lane = pin(args.lane_manifest)
    launcher = pin(root / 'scripts/validation/cert/case_entry.sh')
    command = [launcher['path'], '--case', args.case, '--lane', 'kvc', '--manifest', lane['path'],
               '--manifest-sha256', lane['sha256'], '--out', str(args.run_out.resolve())]
    manifest = {'schema_version': 'kv-r5-frozen.v1', 'lane': 'KVC', 'candidate_sha': spec['candidate_sha'],
        'candidate': matching[0], 'build_receipt': pin(args.build_receipt), 'lane_manifest': lane,
        'tools': spec['tools'], 'harness_repo': str(root), 'harness_closure': closure,
        'cases': {args.case: {'command': command, 'out': str(args.run_out.resolve()), 'launcher': launcher}}, 'inputs': []}
    inputs = {str(kv_envelope.pin(item)): pin(item['path']) for item in kv_envelope.pins([
        manifest, spec, kv_envelope.load(args.build_receipt), kv_envelope.load(spec['workflow_origin']['path'])])}
    for path in (args.selection, args.catalog, args.tool_manifest, args.build_receipt.parent / 'validation-only.log',
                 kv_envelope.SCHEMA, *args.input):
        item = pin(path); inputs[item['path']] = item
    manifest['inputs'] = list(inputs.values())
    preflight(manifest)
    with args.out.open('xb') as stream:
        stream.write(checks.compact(manifest) + b'\n')
    return manifest


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--case', required=True, choices=runner.CASES)
    parser.add_argument('--manifest', required=True, type=Path)
    parser.add_argument('--manifest-sha256', required=True)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--preflight', action='store_true')
    args = parser.parse_args(argv)
    checks.require(digest(args.manifest) == args.manifest_sha256, 'CPU execution spec hash mismatch')
    spec = validate_spec(kv_envelope.load(args.manifest))
    checks.require(spec['case'] == args.case, 'CPU entry case mismatch')
    if args.preflight:
        return 0
    return 0 if runner.run(args.manifest, args.out) else 1
