#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Engine-specific KV dispatch registry; unavailable runners fail closed."""
import argparse
import hashlib
import importlib
import json
from pathlib import Path

CASE_LANES = {
    'BC-KV-001': 'KVV', 'BC-KV-002': 'KVV', 'BC-KV-003': 'KVS',
    'BC-KV-004': 'KVS', 'BC-KV-005': 'KVL', 'BC-KV-006': 'KVD',
    'BC-KV-007': 'KVT', 'BC-KV-008': 'KVV', 'BC-KV-009': 'KVV',
    'BC-KV-010': 'KVC', 'BC-KV-011': 'KVC', 'BC-KV-012': 'KVC',
    'BC-KV-013': 'KVC', 'BC-KV-014': 'KVC', 'BC-KV-015': 'KVC',
}
# Each engine lane replaces only its own entry with its reviewed module.
# None of these can fall back to the r4 common-server adapter.
LANE_MODULES = {
    'KVV': 'kv_lanes.kvv',
    'KVS': 'kv_lanes.kvs',
    'KVL': 'kv_lanes.kvl',
    'KVD': 'kv_lanes.kvd',
    'KVT': 'kv_lanes.kvt',
    'KVC': 'kv_lanes.kvc',
}


def require_lane(case, lane):
    if CASE_LANES.get(case) != lane:
        raise ValueError('KV case/lane does not match the registered catalog mapping')


def adapter(case, lane):
    require_lane(case, lane)
    module = importlib.import_module(LANE_MODULES[lane])
    if any(not callable(getattr(module, name, None)) for name in ('main', 'freeze', 'preflight')):
        raise ValueError('KV adapter must implement main(argv), freeze(args) and preflight(manifest)')
    return module


def preflight(manifest):
    cases = manifest.get('cases')
    if not isinstance(cases, dict) or len(cases) != 1:
        raise ValueError('KV execution requires exactly one frozen case')
    case = next(iter(cases))
    module = adapter(case, manifest['lane'])
    import kv_envelope
    kv_envelope.validate(manifest)
    module.preflight(manifest)
    return [case]


def run_case(manifest_path, manifest, case):
    if preflight(manifest) != [case]:
        raise ValueError('KV execution case differs from preflight')
    import manifest_tool
    raw = Path(manifest_path).read_bytes()
    if json.loads(raw, object_pairs_hook=manifest_tool.unique_object) != manifest:
        raise ValueError('KV manifest differs from preflight')
    manifest_hash = hashlib.sha256(raw).hexdigest()
    import time
    import execution_policy
    import execution_evidence
    import case_launcher
    import result_writer
    import kv_envelope
    import stage0_cgroup
    import stage0_process
    out = Path(manifest['cases'][case]['out'])
    if out.exists():
        raise FileExistsError('refusing to reuse a KV attempt directory')
    lifecycle = out.with_name(out.name + '-lifecycle')
    lifecycle.mkdir(exist_ok=False)
    tools = execution_policy.validate_tools(manifest['tools'])
    environment = execution_policy.prepare(tools, lifecycle, {})
    environment.update(CERT_INPUT_MANIFEST=str(Path(manifest_path).resolve()), CERT_CASE_ID=case,
        CERT_PHASE_ROOT=str(lifecycle), CERT_TOOL_BIN=str(lifecycle / 'tool-bin'),
        CERT_CANDIDATE=manifest['candidate']['path'])
    spec = kv_envelope.load(manifest['lane_manifest']['path'])
    cap, grace = spec['timeout_seconds'], spec['cleanup_grace_seconds']
    deadline = time.monotonic() + cap
    group = None
    pending = None
    outcome = None
    try:
        group = stage0_cgroup.create(spec['cgroup_root'])
        (lifecycle / 'cgroup.json').write_text(json.dumps({'parent': spec['cgroup_root'], 'name': group.name}) + '\n')
        execution_evidence.phase(lifecycle, case, 'inputs-ready')
        assignments = [f'{key}={value}' for key, value in sorted(environment.items())]
        command = [tools['python3']['path'], '-I', '-B', str(Path(__file__).with_name('stage0_cgroup_exec.py')),
            str(group.join_fd), '--', tools['env']['path'], '-i', *assignments,
            *manifest['cases'][case]['command']]
        with (lifecycle / 'stdout.log').open('xb') as stdout, (lifecycle / 'stderr.log').open('xb') as stderr:
            outcome = stage0_process.run_bounded(command, cwd=Path(__file__).resolve().parents[3], env={},
                stdout=stdout, stderr=stderr, cap_seconds=cap-grace,
                grace_seconds=min(5.0, (cap-grace)/2), pass_fds=(group.join_fd,))
        if outcome.timed_out or outcome.returncode != 0:
            raise ValueError(f'{case}: KV runner failed or exceeded its frozen timeout')
    except BaseException as exc:
        pending = exc
    finally:
        # Process-group exit is insufficient: detect and terminate escaped
        # descendants in the owned cgroup, then prove that it became empty.
        if group is not None:
            try:
                leaked = group.populated()
                if leaked:
                    group.kill()
                while group.populated() and time.monotonic() < deadline:
                    time.sleep(min(.1, max(0, deadline-time.monotonic())))
                group.close_if_empty()
                (lifecycle / 'cgroup-cleanup.json').write_text(json.dumps({'empty': True, 'leaked': leaked}) + '\n')
                if leaked:
                    raise ValueError('KV process survived launcher exit')
            except BaseException as exc:
                pending = exc
    try:
        if pending is not None:
            raise pending
        if hashlib.sha256(Path(manifest_path).read_bytes()).hexdigest() != manifest_hash:
            raise ValueError('KV manifest changed during execution')
        execution_policy.verify(tools, lifecycle / 'tool-bin')
        kv_envelope.validate(manifest)
        if result_writer.smoke_origin(out):
            record = kv_envelope.load(out / 'results' / (case + '.json'))
            import validate_results
            try:
                validate_results.check_evidence_class(record)
            except result_writer.NonCertificationEvidence:
                pass  # Local smoke inspection never grants certification credit.
            else:
                raise ValueError('KV smoke record lacks permanent intake rejection')
            paths, hashes = record.get('evidence_paths'), record.get('output_hashes')
            if not paths or not isinstance(hashes, dict) or set(paths) != set(hashes):
                raise ValueError('KV smoke output hashes are incomplete')
            for name in paths:
                path = (out / name).resolve()
                if not path.is_relative_to(out.resolve()) or execution_evidence.digest(path) != hashes[name]:
                    raise ValueError('KV smoke output hash mismatch')
        else:
            records = execution_evidence.report_rows(out)
            if len(records) != 1:
                raise ValueError('KV runner must return exactly one record')
            record = records[0]
        if record.get('ID') != case or record.get('candidate_sha') != manifest['candidate_sha'] or record.get('result') != 'PASS':
            raise ValueError('KV runner returned a failed or mismatched record')
        case_launcher.validate_execution_record(record, manifest_hash, tools)
        execution_evidence.phase(lifecycle, case, 'record-validated')
    except BaseException:
        out.mkdir(exist_ok=True)
        (out / 'results').mkdir(exist_ok=True)
        (out / 'results/lane-error.json').write_text(json.dumps({'ID': case, 'result': 'FAIL',
            'reason': 'KV launcher did not validate terminal execution evidence'}) + '\n')
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--case', required=True)
    parser.add_argument('--lane', required=True, choices=sorted(LANE_MODULES))
    args, remaining = parser.parse_known_args()
    try:
        return adapter(args.case, args.lane).main(['--case', args.case, *remaining])
    except ValueError as exc:
        parser.exit(1, f'{exc}\n')


if __name__ == '__main__':
    raise SystemExit(main())
