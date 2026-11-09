#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Prepare the three pinned BC-COMP-002 inputs from one 64-row source.

Run before freeze, not from any validator or record writer. Include the source,
all generated outputs and the receipt in the frozen input manifest.
"""
import argparse
import hashlib
import json
import re
from pathlib import Path

import comp_reference_runner
import manifest_tool


def prepare(source, source_sha, output_dir):
    source = Path(source).resolve(strict=True)
    raw = source.read_bytes()
    if not re.fullmatch(r'[a-f0-9]{64}', source_sha) or hashlib.sha256(raw).hexdigest() != source_sha:
        raise ValueError('reference source hash mismatch')
    rows = [json.loads(line, object_pairs_hook=manifest_tool.unique_object)
            for line in raw.decode('utf-8').splitlines() if line.strip()]
    if len(rows) != 64:
        raise ValueError('reference source requires exactly 64 prompt rows')
    prompts = []
    for row in rows:
        if not isinstance(row, dict) or not isinstance(row.get('prompt'), str) or not row['prompt'].strip():
            raise ValueError('reference rows require a nonempty prompt string')
        prompts.append(row['prompt'])
    if len(set(prompts)) != 64:
        raise ValueError('reference prompts must be distinct')
    output_dir = Path(output_dir)
    output_dir.mkdir(mode=0o700)  # New directory; never replace an existing freeze input.
    paths = {name: output_dir / name for name in ('reference.jsonl', 'reference-sharegpt.json', 'reference-order.jsonl')}
    with paths['reference.jsonl'].open('x', encoding='utf-8') as stream:
        for prompt in prompts:
            stream.write(json.dumps({'prompt': prompt}, ensure_ascii=False, separators=(',', ':')) + '\n')
    with paths['reference-sharegpt.json'].open('x', encoding='utf-8') as stream:
        json.dump([{'conversations': [{'from': 'human', 'value': prompt},
                                     {'from': 'gpt', 'value': 'placeholder'}]} for prompt in prompts],
                  stream, ensure_ascii=False, separators=(',', ':'))
        stream.write('\n')
    with paths['reference-order.jsonl'].open('x', encoding='utf-8') as stream:
        for seq, prompt in enumerate(prompts):
            encoded = prompt.encode('utf-8')
            slot = {'seq': seq, 'prompt_index': seq, 'prompt_sha256_16': hashlib.sha256(encoded).hexdigest()[:16],
                    'prompt_bytes': len(encoded)}
            stream.write(json.dumps(slot, separators=(',', ':')) + '\n')
    def pin(path):
        return {'path': str(path.resolve()), 'sha256': manifest_tool.sha256_file(path)}
    config = {'comparison_cap': 1024, 'prompt_sets': {'reference': pin(paths['reference.jsonl']),
              'reference_sharegpt': pin(paths['reference-sharegpt.json'])},
              'reference_order': pin(paths['reference-order.jsonl'])}
    comp_reference_runner.reference_inputs(config)
    receipt = {'source': {'path': str(source), 'sha256': source_sha}, 'comparison_config': config}
    receipt_path = output_dir / 'receipt.json'
    with receipt_path.open('x', encoding='utf-8') as stream:
        json.dump(receipt, stream, indent=2, sort_keys=True, ensure_ascii=False)
        stream.write('\n')
    return pin(receipt_path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('source', 'source-sha256', 'out-dir'):
        parser.add_argument('--' + name, required=True)
    args = parser.parse_args()
    try:
        receipt = prepare(args.source, args.source_sha256, args.out_dir)
    except (ValueError, OSError, UnicodeError, TypeError) as exc:
        parser.exit(1, f'reference input preparation failed: {exc}\n')
    print(json.dumps(receipt, sort_keys=True))


if __name__ == '__main__':
    main()
