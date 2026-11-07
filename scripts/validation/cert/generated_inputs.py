#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Structural contracts for offline CPU prompt selection, never future hashes.

No arbitrary generator commands: the backend is the receipt-verified shipped
prompts CLI, always using an explicit local JSONL dataset, never a Hub lookup.
Runtime materialization/launcher integration must precede any readiness claim.
"""
import datetime as dt
import hashlib
import json
import math
import os
import signal
import subprocess
import re
from pathlib import Path
import build_roles
from execution_evidence import digest

KIND = 'offline-prompt-selection-v1'
FIELDS = {'id', 'kind', 'generator', 'generator_version', 'seed', 'config', 'dataset', 'tokenizer', 'template'}
PROFILES = {'chat-short', 'chat-medium', 'rag-medium', 'summarize-long', 'code-medium'}
INTEGER_OPTIONS = {'count', 'count_slack', 'max_repeats', 'select_work_limit'}
REAL_OPTIONS = {'isl_target', 'osl_target', 'isl_tolerance', 'osl_tolerance'}


def pin(item):
    if not isinstance(item, dict) or not isinstance(item.get('path'), str):
        raise ValueError('generated input dependency requires a file pin')
    path = Path(item['path']).resolve(strict=True)
    if not path.is_file() or not re.fullmatch(r'[0-9a-f]{64}', str(item.get('sha256', ''))) or digest(path) != item['sha256']:
        raise ValueError('generated input dependency hash mismatch')
    return path


def validate(recipe, receipt_path, sha):
    if not isinstance(recipe, dict) or set(recipe) != FIELDS:
        raise ValueError('structural recipe fields must be exact; future output hashes are forbidden')
    if recipe['kind'] != KIND or not re.fullmatch(r'[a-z][a-z0-9_-]{0,63}', str(recipe['id'])):
        raise ValueError('unsupported deterministic generator kind/id')
    if type(recipe['seed']) is not int or not 0 <= recipe['seed'] < 2**64:
        raise ValueError('generator seed must be an explicit u64')
    build_roles.load_receipt(receipt_path, sha)
    receipt = json.loads(Path(receipt_path).read_text())
    expected = receipt['builds']['measured']['binaries'].get('metrum-ai-bench-cli-prompts')
    if expected is None or recipe['generator'] != expected or recipe['generator_version'] != receipt['version']:
        raise ValueError('generator must be the same-source verified shipped CPU prompts CLI/version')
    pin(recipe['generator'])
    for key in ('dataset', 'tokenizer', 'template'):
        pin(recipe[key])
    if not re.fullmatch(r'[0-9a-f]{40}', str(recipe['tokenizer'].get('revision', ''))):
        raise ValueError('generator tokenizer revision must be pinned')
    config = recipe['config']
    allowed = INTEGER_OPTIONS | REAL_OPTIONS | {'profile'}
    if not isinstance(config, dict) or not set(config) <= allowed or 'count' not in config:
        raise ValueError('unsupported or missing generator configuration')
    for key in INTEGER_OPTIONS & set(config):
        minimum = 0 if key == 'count_slack' else 1
        if type(config[key]) is not int or not minimum <= config[key] < 2**32:
            raise ValueError('generator integer configuration outside fixed domain')
    for key in REAL_OPTIONS & set(config):
        value = config[key]
        try:
            finite = type(value) in (int, float) and math.isfinite(value)
        except OverflowError:
            finite = False
        if not finite or value < 0 or (key.endswith('target') and value == 0):
            raise ValueError('generator length configuration must be finite and positive')
    if 'profile' in config:
        if not isinstance(config['profile'], str) or config['profile'] not in PROFILES or {'isl_target', 'osl_target'} & set(config):
            raise ValueError('generator profile conflicts with explicit length targets')
    elif not {'isl_target', 'osl_target'} <= set(config):
        raise ValueError('generator requires a fixed profile or both length targets')
    return {'stage': 'structural-only', 'recipe_sha256': contract_hash(recipe)}


def contract_hash(recipe):
    return hashlib.sha256(json.dumps(recipe, sort_keys=True, separators=(',', ':'), allow_nan=False).encode()).hexdigest()


def reject_orphan_runtime(manifest):
    if manifest.get('generated_inputs') is None and any(key in manifest for key in
            ('generated_inputs', 'generated_bindings', 'parent_structural_manifest')):
        raise ValueError('runtime bindings require nonempty generated structural entries')


def structural_context(manifest):
    """Validate frozen structural receipts, not a claim about future output bytes."""
    reject_orphan_runtime(manifest)
    entries = manifest.get('generated_inputs')
    if entries is None:
        return None
    if not isinstance(entries, list) or not entries or len(manifest.get('cases', {})) != 1:
        raise ValueError('generated inputs require a nonempty single-case structural contract')
    receipt = manifest['build_receipt']
    pin(receipt)
    recipes = {}
    frozen_pins = {str(Path(item['path']).resolve()): item['sha256'] for item in manifest.get('inputs', [])}
    if frozen_pins.get(str(Path(receipt['path']).resolve())) != receipt['sha256']:
        raise ValueError('structural manifest must freeze its build receipt input')
    for entry in entries:
        if not isinstance(entry, dict) or set(entry) != {'recipe', 'structural'}:
            raise ValueError('generated entry must freeze recipe and structural receipt')
        recipe = entry['recipe']
        status = validate(recipe, receipt['path'], manifest['candidate_sha'])
        if entry['structural'] != status or recipe['id'] in recipes:
            raise ValueError('frozen generator contract/config/seed mismatch or duplicate id')
        if (recipe['tokenizer']['sha256'] != manifest['tokenizer']['sha256']
                or recipe['tokenizer']['revision'] != manifest['tokenizer']['revision']):
            raise ValueError('generator tokenizer differs from frozen case input')
        for key in ('generator', 'dataset', 'tokenizer', 'template'):
            item = recipe[key]
            if frozen_pins.get(str(Path(item['path']).resolve())) != item['sha256']:
                raise ValueError('structural manifest must freeze every generator dependency')
        recipes[recipe['id']] = recipe
    return recipes


def preflight_label(manifest):
    reject_orphan_runtime(manifest)
    structural = ('generated_inputs' in manifest and 'generated_bindings' not in manifest) or 'derived_inputs' in manifest
    return 'STRUCTURAL-PREFLIGHT-OK' if structural else 'PREFLIGHT-OK'


def run_generator(command, directory):
    """Owned CPU-only process group with no ambient credentials/proxy/exporters."""
    env = {'PATH': os.defpath, 'LANG': 'C.UTF-8', 'LC_ALL': 'C.UTF-8',
           'HF_HUB_OFFLINE': '1', 'TRANSFORMERS_OFFLINE': '1', 'NO_BANNER': '1'}
    with (directory / 'generator.log').open('xb') as log:
        process = subprocess.Popen(command, cwd=directory, env=env, start_new_session=True,
                                   stdout=log, stderr=subprocess.STDOUT)
        try:
            code = process.wait(timeout=900)
            if code:
                raise subprocess.CalledProcessError(code, command)
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()


def materialize(recipe, receipt_path, sha, root, expected_contract):
    """Generate twice in fresh directories and bind real hashes before launch.

    This function emits no success binding on any failure; both attempts/logs
    remain. The caller must refuse measurement until it receives runtime-bound.
    """
    status = validate(recipe, receipt_path, sha)
    if status['recipe_sha256'] != expected_contract:
        raise ValueError('generator contract/config/seed drift after structural freeze')
    root = Path(root).resolve()
    root.mkdir(parents=True, exist_ok=False)
    outputs = []
    for name in ('actual', 'rederived'):
        validate(recipe, receipt_path, sha)
        directory = root / name
        directory.mkdir()
        command = argv(recipe, directory)
        (directory / 'command.json').write_text(json.dumps(command, indent=2) + '\n')
        run_generator(command, directory)
        validate(recipe, receipt_path, sha)
        pins = {}
        for filename in ('prompts.jsonl', 'selection-report.json'):
            path = directory / filename
            if path.is_symlink() or not path.is_file() or not path.resolve().is_relative_to(directory):
                raise ValueError('generator must produce fresh regular output files')
            pins[filename] = {'path': str(path), 'sha256': digest(path)}
        outputs.append(pins)
    if outputs[0]['prompts.jsonl']['sha256'] != outputs[1]['prompts.jsonl']['sha256']:
        raise ValueError('generated input deterministic re-derivation mismatch; measurement blocked')
    binding = {'stage': 'runtime-bound', 'candidate_sha': sha, 'recipe_id': recipe['id'],
               'recipe_sha256': expected_contract, 'seed': recipe['seed'],
               'actual': outputs[0], 'rederived': outputs[1],
               'bound_at': dt.datetime.now(dt.timezone.utc).isoformat()}
    # Actual bytes are now known. Never put this future digest in structural data.
    with (root / 'binding.json').open('x') as stream:
        json.dump(binding, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write('\n')
    return binding


def argv(recipe, output_dir):
    """Called only after validate; no URL, credentials, shell or moving revisions."""
    root = Path(output_dir).resolve()
    command = [str(Path(recipe['generator']['path']).resolve()), '--quiet',
               '--local-jsonl', str(Path(recipe['dataset']['path']).resolve()), '--seed', str(recipe['seed']),
               '--output', str(root / 'prompts.jsonl'), '--report', str(root / 'selection-report.json')]
    for key, value in sorted(recipe['config'].items()):
        command.extend(['--' + key.replace('_', '-'), str(value)])
    return command
