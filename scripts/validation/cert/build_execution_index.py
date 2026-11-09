#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Build a reviewable index from already-executed case evidence; never run a case."""
import argparse
import datetime as dt
import json
import os
import tempfile
from pathlib import Path

import execution_intake as intake
import validate_results as validator


def relative_file(root, value):
    if not isinstance(value, str) or not value or Path(value).is_absolute() or '..' in Path(value).parts:
        raise ValueError('proof path must be evidence-root-relative')
    current = root
    for part in Path(value).parts:
        current = current / part
        if current.is_symlink():
            raise ValueError('proof path may not traverse symlinks')
    if not current.is_file() or not current.resolve().is_relative_to(root):
        raise ValueError('proof file is missing or outside evidence root')
    return current


def pin(root, name):
    return {'path': name, 'sha256': intake.digest(relative_file(root, name))}


def build(catalog, selection_path, results, selected_plan, proof_map, candidate, out, *,
          history=None, carry_over_manifest=None, include_post=False, as_of=None):
    results = Path(results).resolve(strict=True)
    root = results.parent.resolve(strict=True)
    if results.name != 'results' or not results.is_dir():
        raise ValueError('results must be an existing results directory')
    out = Path(out)
    if out.resolve().is_relative_to(results):
        raise ValueError('execution index must not be written inside results')
    if out.exists():
        raise FileExistsError('refusing to replace an execution index')
    rows = validator.load_catalog(Path(catalog))
    selection = validator.load_selection(Path(selection_path), Path(catalog), rows, candidate)
    mapping = intake.read_json(proof_map)
    if not isinstance(mapping, dict):
        raise ValueError('proof mapping must be an object keyed by case')
    active = {'PR', 'RC', 'RC-GPU'} | ({'POST'} if include_post else set())
    selected = {case for case in selection['selected'] if rows[case]['Tier'] in active}
    required = {}
    for case in sorted(selected):
        path = relative_file(root, f'results/{case}.json')
        record = intake.read_json(path)
        if not isinstance(record, dict) or record.get('ID') != case:
            raise ValueError(f'{case}: record identity mismatch')
        if record.get('result') == 'PASS' and record.get('candidate_sha') == candidate:
            required[case] = path
    if set(mapping) != set(required):
        raise ValueError('proof mapping must match exactly the fresh selected PASS records')
    entries = []
    for case, record_path in required.items():
        proof = mapping[case]
        if not isinstance(proof, dict) or set(proof) not in ({'manifest', 'phases'}, {'manifest', 'phases', 'structural_manifest'}):
            raise ValueError(f'{case}: invalid proof mapping')
        entry = {'ID': case, 'record_sha256': intake.digest(record_path),
                 'manifest': pin(root, proof['manifest']), 'phases': pin(root, proof['phases'])}
        if 'structural_manifest' in proof:
            entry['structural_manifest'] = pin(root, proof['structural_manifest'])
        entries.append(entry)
    index = {'candidate_sha': candidate, 'selection_sha256': intake.digest(selection_path),
             'catalog_sha256': intake.digest(catalog), 'selected_plan': pin(root, selected_plan), 'entries': entries}
    raw = (json.dumps(index, indent=2, sort_keys=True, allow_nan=False) + '\n').encode()
    import hashlib
    index_hash = hashlib.sha256(raw).hexdigest()
    # Exercise production intake before emitting an exclusive index. The digest
    # still requires independent review; this producer cannot authenticate itself.
    temp_name = None
    try:
        with tempfile.NamedTemporaryFile(dir=root, prefix='.execution-index-check-', delete=False) as stream:
            temp_name = stream.name
            stream.write(raw)
        _, errors = validator.validate(Path(catalog), results, candidate,
            as_of or dt.datetime.now(dt.timezone.utc), history=history, selection=selection,
            include_post=include_post, carry_over_manifest=carry_over_manifest,
            execution_index=Path(temp_name), execution_index_sha256=index_hash,
            selection_sha256=intake.digest(selection_path))
        if errors:
            raise ValueError('production intake rejected assembled index: ' + '; '.join(errors))
        with out.open('xb') as stream:
            stream.write(raw)
    finally:
        if temp_name is not None:
            os.unlink(temp_name)
    return index_hash


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('catalog', 'selection', 'results', 'proof-map', 'out'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--selected-plan', required=True, help='plan path relative to results parent')
    parser.add_argument('--candidate-sha', required=True)
    parser.add_argument('--history', type=Path)
    parser.add_argument('--carry-over-manifest', type=Path)
    parser.add_argument('--stage', choices=('pre-tag', 'full'), default='pre-tag')
    args = parser.parse_args()
    try:
        value = build(args.catalog, args.selection, args.results, args.selected_plan, args.proof_map,
                      args.candidate_sha, args.out, history=args.history,
                      carry_over_manifest=args.carry_over_manifest, include_post=args.stage == 'full')
    except (OSError, ValueError, KeyError, TypeError) as exc:
        parser.exit(1, f'execution index not built: {exc}\n')
    print('Execution index SHA-256 (requires independent review):', value)


if __name__ == '__main__':
    main()
