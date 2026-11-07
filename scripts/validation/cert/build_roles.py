#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Fail-closed frozen-build roles; validation tools are never measurement tools."""
import json
import os
import re
from pathlib import Path
from execution_evidence import digest
import pretag_artifact


def check_build(item, candidate_sha, role, features):
    if not isinstance(candidate_sha, str) or not re.fullmatch(r'[a-f0-9]{40}', candidate_sha):
        raise ValueError('frozen candidate must be a full commit')
    if not isinstance(item, dict):
        raise ValueError(f'{role} build metadata is required')
    if item.get('candidate_sha') != candidate_sha:
        raise ValueError(f'{role} build must use the same frozen candidate')
    if item.get('role') != role or item.get('profile') != 'release':
        raise ValueError(f'{role} build role/release profile mismatch')
    if item.get('features') != features:
        raise ValueError(f'{role} build features must be {features}')
    expected_origin = 'pretag-workflow' if role in ('measured', 'feature-variant-tokenizer') else 'local-cargo-validation'
    if item.get('origin', {}).get('kind') != expected_origin:
        raise ValueError(f'{role} build origin must be {expected_origin}')
    path = Path(item['path']).resolve(strict=True)
    if not path.is_file() or not os.access(path, os.X_OK):
        raise ValueError(f'{role} build must be an executable file')
    if digest(path) != item.get('sha256'):
        raise ValueError(f'{role} build hash mismatch')
    return path


def check_roles(manifest):
    sha = manifest['candidate_sha']
    measured = manifest['candidate']
    validation = manifest.get('tokenizer_validation')
    primary = check_build(measured, sha, 'measured', ['otlp'])
    helper = check_build(validation, sha, 'validation-only', ['otlp', 'tokenizer'])
    if primary.samefile(helper) or measured['sha256'] == validation['sha256']:
        raise ValueError('measured and validation-only builds must be distinct')
    return primary, helper


def load_receipt(path, sha):
    path = Path(path).resolve(strict=True)
    receipt = json.loads(path.read_text())
    if receipt.get('candidate_sha') != sha:
        raise ValueError('build receipt must match frozen candidate')
    builds = receipt['builds']
    measured = builds['measured']['binaries']
    validation = builds['validation-only']['binaries']['metrum-ai-bench-cli-llm']
    if not measured:
        raise ValueError('measured binary registry is empty')
    origin = builds['measured']['origin']
    if receipt.get('version') != origin.get('version'):
        raise ValueError('receipt version must match workflow archive')
    expected = pretag_artifact.verify(origin, sha)
    if set(measured) != set(expected):
        raise ValueError('measured registry must match workflow archive binaries')
    for name, item in measured.items():
        check_roles({'candidate_sha': sha, 'candidate': item, 'tokenizer_validation': validation})
        if item['sha256'] != expected[name] or item.get('origin') != origin:
            raise ValueError('measured binary differs from verified workflow archive')
    entry = builds['validation-only']
    command = entry['argv']
    if command[:2] != ['cargo', 'build'] or command.count('--features') != 1:
        raise ValueError('validation receipt must record local cargo build')
    if command[command.index('--features') + 1] != 'otlp,tokenizer' or '--release' not in command or '--locked' not in command:
        raise ValueError('validation build command/profile mismatch')
    if digest(path.parent / 'validation-only.log') != entry['log_sha256']:
        raise ValueError('validation build log hash mismatch')
    return list(measured.values()), validation


def load_variant(path, sha):
    """A distinct workflow-built role, never silently promoted validation output."""
    measured, validation = load_receipt(path, sha)
    receipt = json.loads(Path(path).read_text())
    entry = receipt['builds'].get('feature-variant-tokenizer')
    if not isinstance(entry, dict):
        raise ValueError('BC-GPU-010 requires a workflow tokenizer feature variant')
    origin = entry['origin']
    if (origin.get('run_id') != measured[0]['origin']['run_id']
            or origin.get('version') != receipt['version']):
        raise ValueError('variant must share the frozen pretag run/version')
    expected = pretag_artifact.verify(origin, sha, variant=True)
    binaries = entry['binaries']
    if set(binaries) != {'metrum-ai-bench-cli-llm'} or set(expected) != set(binaries):
        raise ValueError('variant registry must contain only the verified LLM CLI')
    item = binaries['metrum-ai-bench-cli-llm']
    binary = check_build(item, sha, 'feature-variant-tokenizer', ['otlp', 'tokenizer'])
    if (item['sha256'] != expected['metrum-ai-bench-cli-llm'] or item.get('origin') != origin
            or any(item['sha256'] == m['sha256'] or binary.samefile(m['path']) for m in measured)
            or item['sha256'] == validation['sha256'] or binary.samefile(validation['path'])):
        raise ValueError('feature variant differs from workflow or aliases shipped/validation build')
    return item


def check_manifest(manifest):
    primary, helper = check_roles(manifest)
    receipt = manifest['build_receipt']
    if digest(Path(receipt['path'])) != receipt['sha256']:
        raise ValueError('build receipt hash mismatch')
    measured, validation = load_receipt(receipt['path'], manifest['candidate_sha'])
    if (manifest['candidate'] not in measured or manifest['tokenizer_validation'] != validation
            or manifest.get('measured_binaries') != measured):
        raise ValueError('manifest build roles differ from execution-time receipt')
    reject_validation_use(manifest['cases'], validation)
    lane = manifest['lane_manifest']
    if digest(Path(lane['path'])) != lane['sha256']:
        raise ValueError('lane manifest hash mismatch')
    check_measured_config(json.loads(Path(lane['path']).read_text()), measured, validation)
    return primary, helper


MEASURED_CLI_FIELDS = {'strategic_binary', 'vlm_binary', 'modality_binary', 'prompt_binary',
                       'llm_binary', 'asr_binary', 'imagegen_binary', 'candidate_binary'}


def check_measured_config(value, measured, validation):
    reject_validation_use(value, validation)
    registered = {item['sha256'] for item in measured}
    def walk(node):
        if isinstance(node, dict):
            for key, child in node.items():
                if key in MEASURED_CLI_FIELDS or (key.endswith('_binary') and key != 'all_smi_binary'):
                    if not isinstance(child, dict) or child.get('sha256') not in registered:
                        raise ValueError(f'{key} must be in the measured build registry')
                    path = Path(child['path'])
                    if not path.is_file() or not os.access(path, os.X_OK) or digest(path) != child['sha256']:
                        raise ValueError(f'{key} measured artifact hash/executable mismatch')
                walk(child)
        elif isinstance(node, list):
            for child in node:
                walk(child)
    walk(value)


def reject_validation_use(value, validation):
    """Check measured command/config pins, not the validation input registry."""
    if isinstance(value, dict):
        if value.get('sha256') == validation['sha256']:
            raise ValueError('validation-only build cannot be a measured input executable')
        for child in value.values():
            reject_validation_use(child, validation)
    elif isinstance(value, list):
        for child in value:
            reject_validation_use(child, validation)
    elif isinstance(value, str):
        path = Path(value)
        # argv values are inspected individually, without interpreting a shell.
        if path.is_absolute() and path.is_file():
            if path.samefile(validation['path']) or (os.access(path, os.X_OK) and digest(path) == validation['sha256']):
                raise ValueError('validation-only build cannot appear in measured argv/config')
