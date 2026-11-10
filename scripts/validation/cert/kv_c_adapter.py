# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""SGLang/LMCache adapters for the shared kv-r5-frozen.v1 envelope."""
import argparse
import json
import os
import signal
import socket
import subprocess
from pathlib import Path

import build_roles
import execution_policy
import harness_integrity
import kv_external_common as common
import manifest_tool
import result_writer

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
SCHEMA = ROOT / 'schemas/kv-r5-frozen.v1.json'


def load(path):
    return json.loads(Path(path).read_text(), object_pairs_hook=manifest_tool.unique_object)


def file_pin(path):
    path = Path(path).resolve(strict=True)
    return {'path': str(path), 'sha256': manifest_tool.sha256_file(path)}


def pins(value):
    if isinstance(value, dict):
        if 'path' in value and 'sha256' in value:
            yield value
        for child in value.values():
            yield from pins(child)
    elif isinstance(value, list):
        for child in value:
            yield from pins(child)


def validate_spec(spec, family, lane, case=None):
    if spec.get('schema_version') != 'kv-r5-c-execution.v1' or not manifest_tool.REV.fullmatch(str(spec.get('candidate_sha', ''))):
        raise ValueError('full candidate and lane execution schema required')
    case = case or spec.get('case')
    if case not in family.CASES or spec.get('case') != case:
        raise ValueError('one matching engine case required')
    config = spec['lanes'][lane]
    if config.get('kv_case') != case:
        raise ValueError('case/config mismatch')
    for item in pins(spec):
        if not Path(item['path']).is_absolute() or not manifest_tool.SHA.fullmatch(str(item['sha256'])):
            raise ValueError('absolute hash-pinned inputs required')
        common.pinned(item)
    for key in ('candidate', 'baseline'):
        path = common.pinned(spec[key])
        if not os.access(path, os.X_OK) or spec.get(key + '_binary_sha256') != spec[key]['sha256']:
            raise ValueError('execution binary hash or executable mismatch')
    tools = execution_policy.validate_case_tools(spec['tools'], lane)
    if execution_policy.load_manifest(common.pinned(spec['tool_manifest'])) != tools:
        raise ValueError('tool manifest differs from actual tools')
    if not isinstance(config.get('server_flags'), list) or any(not isinstance(v, str) or not v or '\0' in v for v in config['server_flags']):
        raise ValueError('server flags must be a nonempty-string argv array')
    if type(config.get('port')) is not int or not 1024 <= config['port'] <= 65535:
        raise ValueError('invalid server port')
    fit = family.validate_config(config)
    if type(spec.get('timeout_seconds')) is not int or not 180 <= spec['timeout_seconds'] <= 14400:
        raise ValueError('explicit bounded setup/workload/cleanup envelope required')
    grace = spec.get('cleanup_grace_seconds')
    if type(grace) is not int or not 120 <= grace < spec['timeout_seconds'] - 120:
        raise ValueError('reserve scoped container cleanup inside the outer cgroup cleanup envelope')
    group = spec.get('cgroup_root')
    if not isinstance(group, str) or not Path(group).is_absolute() or '..' in Path(group).parts or str(Path(group)) != group:
        raise ValueError('canonical delegated cgroup root required')
    root = Path(spec['model_snapshot']).absolute()
    hub = Path(spec['hub_cache']).resolve(strict=True)
    if not root.is_dir() or root.name != config['revision'] or not root.resolve().is_relative_to(hub):
        raise ValueError('offline model snapshot must bind the exact revision inside hub cache')
    files = spec['model_files']
    named_files = [p for p in root.rglob('*') if p.is_file()]
    inventory = {str(p.resolve()) for p in named_files}
    if not inventory or len(files) != len(inventory) or {p['path'] for p in files} != inventory:
        raise ValueError('complete model snapshot file/checksum lock required')
    if any(not common.pinned(p).is_relative_to(hub) for p in files):
        raise ValueError('model file resolves outside staged hub cache')
    if not any(p.suffix == '.safetensors' for p in named_files) or not (root / 'tokenizer.json').is_file():
        raise ValueError('model weights and tokenizer required')
    # Independently verify every tokenized cohort fits the demonstrated runtime.
    # The pinned per-prompt counts are inputs, not estimates from string length.
    lengths = load(common.pinned(spec['prompt_token_counts']))
    if lengths.get('tokenizer_sha256') != manifest_tool.sha256_file(root / 'tokenizer.json'):
        raise ValueError('prompt length evidence uses another tokenizer')
    import hashlib
    actual = {hashlib.sha256(text.encode()).hexdigest() for cohort in config['prompt_sets'] for text in common.prompts(config, cohort)[1]}
    counts = lengths.get('counts', {})
    if set(counts) != actual or any(type(n) is not int or not 0 < n <= fit['max_prompt_tokens'] for n in counts.values()):
        raise ValueError('cohort token-count coverage or fit bounds differ')
    if not manifest_tool.REV.fullmatch(str(config.get('revision', ''))):
        raise ValueError('exact model revision required')
    return config


def preflight(manifest, family, lane):
    result_writer.require_python()
    if manifest.get('schema_version') != 'kv-r5-frozen.v1' or manifest.get('lane') != lane:
        raise ValueError('KV frozen envelope/lane mismatch')
    import kv_envelope
    kv_envelope.validate(manifest, lane)
    cases = manifest['cases']
    if not isinstance(cases, dict) or len(cases) != 1 or next(iter(cases)) not in family.CASES:
        raise ValueError('one registered engine case required')
    case = next(iter(cases))
    for item in manifest['inputs']:
        common.pinned(item)
    spec_path = common.pinned(manifest['lane_manifest'])
    spec = load(spec_path)
    validate_spec(spec, family, lane, case)
    if spec['candidate_sha'] != manifest['candidate_sha']:
        raise ValueError('candidate identity differs')
    measured, _ = build_roles.load_receipt(common.pinned(manifest['build_receipt']), manifest['candidate_sha'])
    matches = [item for item in measured if item['path'] == spec['candidate']['path'] and item['sha256'] == spec['candidate']['sha256']]
    if len(matches) != 1 or manifest['candidate'] != matches[0]:
        raise ValueError('candidate is not the shipped measured workflow artifact')
    if manifest.get('harness_repo') != str(ROOT):
        raise ValueError('frozen harness repository differs')
    closure = harness_integrity.verify(ROOT, manifest['candidate_sha'])
    if manifest['harness_closure'] != closure:
        raise ValueError('candidate harness closure changed')
    entry = cases[case]
    expected = [str(HERE / 'case_entry.sh'), '--case', case, '--lane', lane.lower(), '--manifest', str(spec_path),
                '--manifest-sha256', manifest['lane_manifest']['sha256'], '--out', entry['out']]
    if entry['command'] != expected or common.pinned(entry['launcher']) != HERE / 'case_entry.sh':
        raise ValueError('frozen command differs from production case entry')
    if manifest['tools'] != spec['tools']:
        raise ValueError('frozen execution tools differ')
    required = {str(common.pinned(p)) for p in [*pins(spec), *pins(load(common.pinned(spec['lanes'][lane]['fit']))), *closure]}
    present = {str(common.pinned(p)) for p in manifest['inputs']}
    if not required <= present:
        raise ValueError('missing transitive frozen inputs')
    for p in present:
        result_writer.require_input_origin(entry['out'], Path(p).parent)
    return list(cases)


def freeze(args, family, lane):
    selection, spec = load(args.selection), load(args.lane_manifest)
    if (selection['candidate_sha'] != spec['candidate_sha'] or args.case not in selection['selected']
            or selection['catalog_sha256'] != manifest_tool.sha256_file(args.catalog)):
        raise ValueError('case/candidate/catalog not selected together')
    validate_spec(spec, family, lane, args.case)
    if common.pinned(spec['candidate']) != args.candidate.resolve() or common.pinned(spec['baseline']) != args.baseline.resolve():
        raise ValueError('freeze binary arguments differ from execution spec')
    if common.pinned(spec['tool_manifest']) != args.tool_manifest.resolve():
        raise ValueError('freeze tools differ from execution spec')
    if getattr(args, 'generated_recipes', None) is not None or args.command_json is not None:
        raise ValueError('engine freeze requires concrete pinned inputs and fixed dispatch')
    measured, _ = build_roles.load_receipt(args.build_receipt, spec['candidate_sha'])
    matches = [item for item in measured if item['path'] == spec['candidate']['path'] and item['sha256'] == spec['candidate']['sha256']]
    if len(matches) != 1:
        raise ValueError('candidate/build receipt differs')
    closure = harness_integrity.verify(ROOT, spec['candidate_sha'])
    inputs = {}
    for value in [spec, load(args.build_receipt), load(common.pinned(spec['lanes'][lane]['fit'])), closure]:
        for item in pins(value):
            path = common.pinned(item)
            inputs[str(path)] = file_pin(path)
    for path in [args.selection, args.catalog, args.lane_manifest, args.build_receipt,
                 args.build_receipt.parent / 'validation-only.log', SCHEMA, *args.input]:
        item = file_pin(path)
        inputs[item['path']] = item
    lane_pin = file_pin(args.lane_manifest)
    command = [str(HERE / 'case_entry.sh'), '--case', args.case, '--lane', lane.lower(), '--manifest', lane_pin['path'],
               '--manifest-sha256', lane_pin['sha256'], '--out', str(args.run_out.resolve())]
    manifest = {'schema_version': 'kv-r5-frozen.v1', 'lane': lane, 'candidate_sha': spec['candidate_sha'], 'candidate': matches[0],
                'build_receipt': file_pin(args.build_receipt), 'lane_manifest': lane_pin, 'tools': spec['tools'],
                'inputs': list(inputs.values()), 'harness_repo': str(ROOT), 'harness_closure': closure,
                'cases': {args.case: {'launcher': file_pin(HERE / 'case_entry.sh'), 'command': command, 'out': str(args.run_out.resolve())}}}
    preflight(manifest, family, lane)
    with args.out.open('x') as stream:
        json.dump(manifest, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write('\n')
    return manifest


def main(argv, family, lane):
    parser = argparse.ArgumentParser()
    parser.add_argument('--case', required=True, choices=sorted(family.CASES))
    parser.add_argument('--manifest', required=True, type=Path)
    parser.add_argument('--manifest-sha256', required=True)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--preflight', action='store_true')
    args = parser.parse_args(argv)
    if manifest_tool.sha256_file(args.manifest) != args.manifest_sha256:
        raise ValueError('execution spec hash mismatch')
    spec = load(args.manifest)
    validate_spec(spec, family, lane, args.case)
    if args.preflight:
        return 0
    # A real launch must come from the generic frozen-envelope launcher, whose
    # input binding is consumed by result_writer and checked again at intake.
    envelope_path = os.environ.get('CERT_INPUT_MANIFEST')
    if not envelope_path:
        raise ValueError('guarded frozen-envelope launcher required')
    envelope = load(envelope_path)
    preflight(envelope, family, lane)
    if common.pinned(envelope['lane_manifest']) != args.manifest.resolve() or envelope['cases'][args.case]['out'] != str(args.out):
        raise ValueError('launch arguments differ from frozen envelope')
    env = os.environ.copy()
    env.update(CERT_CASE_ID=args.case, CERT_HF_HUB_CACHE=spec['hub_cache'])
    command = [spec['tools']['bash']['path'], str(HERE / f'lane-{lane.lower()}.sh'),
               '--candidate', spec['candidate']['path'], '--baseline', spec['baseline']['path'],
               '--manifest', str(args.manifest), '--out', str(args.out)]
    process = subprocess.Popen(command, env=env, start_new_session=True)
    try:
        try:
            code = process.wait(timeout=spec['timeout_seconds'] - spec['cleanup_grace_seconds'] - 120)
        except subprocess.TimeoutExpired:
            # Reserve cleanup inside the case envelope. The shell's existing
            # owner-labelled cleanup runs on TERM; a timeout cannot certify.
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=120)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=10)
            return 124
        if manifest_tool.sha256_file(args.manifest) != args.manifest_sha256:
            raise ValueError('execution spec changed during launch')
        if code == 0:
            owner = (args.out / 'evidence/server-container-owner.txt').read_text().strip()
            if len(owner) != 32 or any(c not in '0123456789abcdef' for c in owner):
                raise ValueError('cleanup ownership receipt missing')
            remaining = subprocess.run([spec['tools']['docker']['path'], 'ps', '-a', '--filter',
                                       'label=metrum.cert.owner=' + owner, '--format', '{{.ID}}'],
                                      check=True, capture_output=True, text=True, timeout=30)
            if remaining.stdout.strip():
                raise ValueError('owned container remains after shell cleanup')
            config = spec['lanes'][lane]
            ports = [config['port'], config['proxy_port']] + ([config['metrics_port']] if 'metrics_port' in config else [])
            for port in ports:
                with socket.socket() as probe:
                    probe.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                    probe.bind(('127.0.0.1', port))
            (args.out / 'evidence/kv-cleanup-reconciled.json').write_text(json.dumps(
                {'owned_containers': [], 'loopback_ports_bindable': ports}) + '\n')
        return code
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=120)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=10)


def field_main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--lane', choices=['KVS', 'KVL'], required=True)
    parser.add_argument('--field', required=True)
    args = parser.parse_args()
    import importlib
    family = importlib.import_module('kv_sglang_runner' if args.lane == 'KVS' else 'kv_lmcache_runner')
    spec = load(args.manifest)
    # Full transitive preflight already occurred before shell entry. Recheck
    # small manifest/binary/config pins here; full model hashing per field would
    # otherwise repeat for every shell query.
    for key in ('candidate', 'baseline'):
        common.pinned(spec[key])
    config = spec['lanes'][args.lane]
    family.validate_config(config)
    value = spec['candidate_sha'] if args.field == 'candidate_sha' else config[args.field]
    if isinstance(value, (dict, list)):
        raise ValueError('scalar field required')
    print(value)


if __name__ == '__main__':
    field_main()
