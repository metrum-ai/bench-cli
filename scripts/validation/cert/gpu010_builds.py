#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Exact per-cell GPU010 build policy; never a global feature-profile override."""
import json
import os
from pathlib import Path
import build_roles
from execution_evidence import digest

CELLS = ('truncated', 'empty', 'missing', 'fallback')
VARIANT_KEY = 'feature_variant_tokenizer'
ORIGIN_PINS = ('run', 'artifact_metadata', 'artifact_listing', 'archive', 'checksum', 'download_log')


def bindings(manifest):
    if set(manifest.get('cases', {})) != {'BC-GPU-010'}:
        raise ValueError('tokenizer feature variant is scoped exclusively to BC-GPU-010 fallback')
    if not manifest.get(VARIANT_KEY):
        raise ValueError('BC-GPU-010 explicit frozen feature-variant binding required')
    build_roles.check_manifest(manifest)
    receipt = manifest['build_receipt']
    variant = build_roles.load_variant(receipt['path'], manifest['candidate_sha'])
    if manifest.get(VARIANT_KEY) != variant:
        raise ValueError('GPU010 feature variant must match the frozen workflow registry')
    data = json.loads(Path(receipt['path']).read_text())
    shipped = data['builds']['measured']['binaries']['metrum-ai-bench-cli-llm']
    if manifest['candidate'] != shipped:
        raise ValueError('GPU010 primary must be the shipped LLM CLI')
    pins = {str(Path(item['path']).resolve()): item['sha256'] for item in manifest.get('inputs', [])}
    required = [receipt, variant] + [variant['origin'][key] for key in ORIGIN_PINS]
    for item in required:
        path = Path(item['path']).resolve(strict=True)
        if pins.get(str(path)) != item['sha256'] or digest(path) != item['sha256']:
            raise ValueError('GPU010 variant executable/receipt/provenance must be frozen input pins')
    return {name: variant if name == 'fallback' else shipped for name in CELLS}


def validate_record(manifest, record, out):
    """Replay every unchanged criterion and bind raw commands to the four builds."""
    import g4_edge_checks
    bindings(manifest)
    versions = record.get('versions', {})
    if (record.get('ID') != 'BC-GPU-010' or record.get('candidate_sha') != manifest['candidate_sha']
            or record.get('result') != 'PASS'
            or versions.get('candidate_binary_sha256') != manifest['candidate']['sha256']
            or versions.get('feature_variant_binary_sha256') != manifest[VARIANT_KEY]['sha256']
            or versions.get('feature_variant_label') != 'feature-variant build (tokenizer)'):
        raise ValueError('GPU010 result build/source/version labels mismatch')
    expected_thresholds = {'requests_per_live_cell': 4, 'empty_issuance_count': 0, 'legacy_rate_tolerance_pct': 1}
    thresholds = record.get('thresholds', {})
    if thresholds != expected_thresholds or any(type(n) is not int for n in thresholds.values()):
        raise ValueError('GPU010 thresholds changed')
    observed = record.get('observed_values', {})
    cells = observed.get('cells', {})
    if set(cells) != set(CELLS) or observed.get('failed_cells') != []:
        raise ValueError('GPU010 requires every unchanged cell to pass')
    evidence = Path(out).resolve() / 'evidence'
    def read(name, suffix, optional=False):
        path = evidence / ('g4-edge-' + name + '-' + suffix)
        if not path.resolve().is_relative_to(evidence.resolve()):
            raise ValueError('GPU010 unsafe evidence path')
        if optional and not path.exists():
            return ''
        return path.read_text()
    for name in CELLS:
        binary, proof = cell(manifest, name)
        if json.loads(read(name, 'build.json')) != proof:
            raise ValueError('GPU010 per-cell pre-request build proof mismatch')
        command = json.loads(read(name, 'command.json'))
        if (not isinstance(command, list) or not command or not all(isinstance(s, str) for s in command)
                or Path(command[0]).resolve() != binary):
            raise ValueError('GPU010 raw command does not use its per-cell build')
        required_flags = {'--num-requests': '4', '--concurrency': '1', '--max-tokens': '128'}
        if name == 'fallback':
            required_flags['--tokenizer'] = manifest['tokenizer']['path']
        elif '--tokenizer' in command:
            raise ValueError('GPU010 shipped cells must not request tokenizer fallback')
        if name == 'empty':
            required_flags['--stop-after-seconds'] = '0'
        elif '--stop-after-seconds' in command:
            raise ValueError('GPU010 live cells must not disable issuance')
        for flag, value in required_flags.items():
            if command.count(flag) != 1:
                raise ValueError('GPU010 raw command changes fixed cell configuration')
            actual = command[command.index(flag) + 1:command.index(flag) + 2]
            if flag == '--tokenizer':
                if (len(actual) != 1 or Path(actual[0]).resolve() != Path(value).resolve()
                        or digest(actual[0]) != manifest['tokenizer']['sha256']):
                    raise ValueError('GPU010 fallback tokenizer differs from frozen input')
            elif actual != [value]:
                raise ValueError('GPU010 raw command changes fixed cell configuration')
        outcome = json.loads(read(name, 'outcome.json'))
        if set(outcome) != {'exit_code'} or type(outcome['exit_code']) is not int:
            raise ValueError('GPU010 raw exit outcome missing')
        detail = g4_edge_checks.check_cell(name, read(name, 'requests.jsonl', True),
                                         read(name, 'trace.jsonl', True), outcome['exit_code'])
        if cells[name] != {**detail, 'build': proof}:
            raise ValueError('GPU010 observed cell differs from raw-evidence replay')


def from_environment(args):
    path = os.environ.get('CERT_INPUT_MANIFEST')
    if os.environ.get('CERT_CASE_ID') != 'BC-GPU-010' or not path:
        raise ValueError('GPU010 requires its frozen per-case launch environment')
    manifest = json.loads(Path(path).read_text())
    if (manifest['candidate_sha'] != args.sha
            or Path(manifest['candidate']['path']).resolve() != args.candidate.resolve()
            or Path(manifest['lane_manifest']['path']).resolve() != args.manifest.resolve()):
        raise ValueError('GPU010 runner arguments differ from frozen per-case manifest')
    bindings(manifest)  # Validate all builds BEFORE the first measured request.
    return manifest


def cell(manifest, name):
    if name not in CELLS:
        raise ValueError('unknown GPU010 cell')
    item = bindings(manifest)[name]
    label = 'feature-variant build (tokenizer)' if name == 'fallback' else 'shipped build (otlp)'
    proof = {'cell': name, 'label': label, 'role': item['role'], 'features': item['features'],
             'profile': item['profile'], 'candidate_sha': item['candidate_sha'],
             'binary_sha256': item['sha256'], 'build_receipt_sha256': manifest['build_receipt']['sha256'],
             'workflow_run_id': item['origin']['run_id'], 'artifact_name': item['origin']['artifact_name']}
    return Path(item['path']).resolve(strict=True), proof
