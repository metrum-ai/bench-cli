#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Bind local-smoke's own L1 selection before its first measured invocation."""
import argparse
import json
import os
import subprocess
from pathlib import Path
import build_roles
import generated_inputs as generated
import manifest_tool
from execution_evidence import digest


def expected(manifest):
    if set(manifest.get('cases', {})) != {'BC-LIVE-001'} or manifest.get('lane') != 'L1':
        raise ValueError('L1 derived selection requires its exact single-case contract')
    build_roles.check_manifest(manifest)
    receipt = json.loads(Path(manifest['build_receipt']['path']).read_text(), object_pairs_hook=manifest_tool.unique_object)
    generator = receipt['builds']['measured']['binaries']['metrum-ai-bench-cli-prompts']
    cfg = json.loads(Path(manifest['lane_manifest']['path']).read_text(), object_pairs_hook=manifest_tool.unique_object)['lanes']['L1']
    if manifest['tokenizer'].get('revision') != cfg.get('revision'):
        raise ValueError('L1 tokenizer revision must match the frozen model revision')
    template = cfg.get('template')
    generated.pin(template)
    generated.pin(manifest['tokenizer'])
    return {'kind': 'l1-prompt-selection-v1', 'source_slot': 'default', 'generator': generator,
            'generator_version': receipt['version'], 'seed': 7,
            'config': {'count': 16, 'profile': 'chat-short'},
            'tokenizer': manifest['tokenizer'], 'template': template}


def declaration(manifest):
    contract = expected(manifest)
    return {'contract': contract, 'contract_sha256': generated.contract_hash(contract)}


def validate(manifest):
    value = declaration(manifest)
    if manifest.get('derived_inputs') != value:
        raise ValueError('L1 selection needs its frozen generator/version/config/template/seed contract')
    frozen = {str(Path(item['path']).resolve()): item['sha256'] for item in manifest.get('inputs', [])}
    for key in ('generator', 'tokenizer', 'template'):
        item = value['contract'][key]
        if frozen.get(str(Path(item['path']).resolve())) != item['sha256']:
            raise ValueError('L1 derived dependency must be a frozen input pin')
    return value


def resolved_recipe(manifest):
    contract = validate(manifest)['contract']
    cfg = json.loads(Path(manifest['lane_manifest']['path']).read_text(), object_pairs_hook=manifest_tool.unique_object)['lanes']['L1']
    dataset = cfg['prompt_sets']['default']
    generated.pin(dataset)  # Default runtime must already be concrete.
    recipe = {'id': 'l1-derived', 'kind': generated.KIND, 'dataset': dataset}
    recipe.update({key: contract[key] for key in ('generator', 'generator_version', 'seed', 'config', 'tokenizer', 'template')})
    generated.validate(recipe, manifest['build_receipt']['path'], manifest['candidate_sha'])
    return recipe


def check_command(recipe, command_path, input_path):
    command = json.loads(Path(command_path).read_text(), object_pairs_hook=manifest_tool.unique_object)
    if Path(command['binary']).resolve() != Path(recipe['generator']['path']).resolve():
        raise ValueError('L1 generator is not the frozen shipped CPU CLI')
    values = command['argv']
    required = {'--count': '16', '--seed': '7', '--profile': 'chat-short',
                '--local-jsonl': recipe['dataset']['path'], '--output': str(input_path),
                '--report': str(input_path.parent / 'prompt-mix.json')}
    if not isinstance(values, list) or len(values) != 1 + 2 * len(required) or values.count('--quiet') != 1:
        raise ValueError('L1 generator arguments changed')
    for flag, value in required.items():
        if values.count(flag) != 1 or values[values.index(flag) + 1:values.index(flag) + 2] != [value]:
            raise ValueError('L1 generator config/seed/input differs from structural contract')


def boundary(manifest, command_path, input_path, out):
    recipe = resolved_recipe(manifest)
    case_out = Path(manifest['cases']['BC-LIVE-001']['out']).resolve()
    input_path = Path(input_path).resolve(strict=True)
    out = Path(out).resolve()
    if input_path != case_out / 'evidence/l1/prompts.jsonl' or out != case_out / 'evidence/l1/input-binding':
        raise ValueError('L1 derived output must be inside its fresh case evidence')
    check_command(recipe, command_path, input_path)
    status = generated.materialize(recipe, manifest['build_receipt']['path'], manifest['candidate_sha'],
                                   out, generated.contract_hash(recipe))
    if digest(input_path) != status['actual']['prompts.jsonl']['sha256']:
        raise ValueError('L1 original selection differs from independent re-derivation; no measured request allowed')
    receipt = {'contract_sha256': manifest['derived_inputs']['contract_sha256'],
               'input': {'path': str(input_path), 'sha256': digest(input_path)},
               'command': {'path': str(Path(command_path).resolve()), 'sha256': digest(command_path)},
               'binding': {'path': str(out / 'binding.json'), 'sha256': digest(out / 'binding.json')}}
    with (out / 'original-binding.json').open('x') as stream:
        json.dump(receipt, stream, indent=2, sort_keys=True)
        stream.write('\n')
    phase_root = os.environ.get('CERT_PHASE_ROOT')
    if phase_root:
        from execution_evidence import phase
        phase(phase_root, 'BC-LIVE-001', 'inputs-ready')
    return receipt


def verify(manifest, out):
    recipe = resolved_recipe(manifest)
    if Path(out).resolve() != Path(manifest['cases']['BC-LIVE-001']['out']).resolve():
        raise ValueError('L1 derived evidence belongs to a different case output')
    root = Path(out).resolve() / 'evidence/l1'
    receipt_path = root / 'input-binding/original-binding.json'
    receipt = json.loads(receipt_path.read_text(), object_pairs_hook=manifest_tool.unique_object)
    if receipt.get('contract_sha256') != manifest['derived_inputs']['contract_sha256']:
        raise ValueError('L1 derived receipt contract mismatch')
    expected_paths = {'input': root / 'prompts.jsonl', 'command': root / 'prompt-command.json',
                      'binding': root / 'input-binding/binding.json'}
    for key, path in expected_paths.items():
        if Path(receipt[key]['path']).resolve() != path:
            raise ValueError('L1 derived receipt output path mismatch')
        generated.pin(receipt[key])
    check_command(recipe, expected_paths['command'], expected_paths['input'])
    binding = json.loads(expected_paths['binding'].read_text(), object_pairs_hook=manifest_tool.unique_object)
    if (binding.get('stage') != 'runtime-bound' or binding.get('candidate_sha') != manifest['candidate_sha']
            or binding.get('recipe_id') != recipe['id'] or binding.get('recipe_sha256') != generated.contract_hash(recipe)
            or type(binding.get('seed')) is not int or binding['seed'] != recipe['seed']):
        raise ValueError('L1 derived runtime source/config/seed mismatch')
    for attempt in ('actual', 'rederived'):
        directory = root / 'input-binding' / attempt
        if set(binding[attempt]) != {'prompts.jsonl', 'selection-report.json'}:
            raise ValueError('L1 derived output registry mismatch')
        for filename, item in binding[attempt].items():
            if Path(item['path']).resolve() != directory / filename:
                raise ValueError('L1 derived output outside owned generation directory')
            generated.pin(item)
        if json.loads((directory / 'command.json').read_text(), object_pairs_hook=manifest_tool.unique_object) != generated.argv(recipe, directory):
            raise ValueError('L1 re-derived command differs from frozen configuration')
        if not (directory / 'generator.log').is_file():
            raise ValueError('L1 derived generator log missing')
    actual = binding['actual']['prompts.jsonl']['sha256']
    if actual != receipt['input']['sha256'] or actual != binding['rederived']['prompts.jsonl']['sha256']:
        raise ValueError('L1 generated input hashes differ')
    return {'input_sha256': actual, 'rederived_sha256': binding['rederived']['prompts.jsonl']['sha256'],
            'recipe_sha256': binding['recipe_sha256'], 'binding_sha256': digest(receipt_path),
            'bound_at': binding['bound_at']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ('manifest', 'command', 'input', 'out'):
        parser.add_argument('--' + key, type=Path, required=True)
    args = parser.parse_args()
    try:
        if os.environ.get('CERT_CASE_ID') != 'BC-LIVE-001':
            raise ValueError('L1 derived hook requires the exact selected case environment')
        boundary(json.loads(args.manifest.read_text(), object_pairs_hook=manifest_tool.unique_object), args.command, args.input, args.out)
    except (ValueError, KeyError, OSError, RuntimeError, subprocess.SubprocessError) as exc:
        parser.exit(1, f'derived input binding failed: {exc}\n')


if __name__ == '__main__':
    main()
