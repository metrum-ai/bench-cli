#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""One offline-preflight/launch entry for a complete selected GPU manifest set."""
import argparse
import csv
import datetime as dt
import json
import re
import subprocess
from pathlib import Path
import case_launcher
import manifest_tool
from execution_evidence import digest, require_python


def approved_na_cases(index, selected_gpu, candidate):
    """Validate pinned N/A records without converting them into executions."""
    import validate_results
    items = index.get('approved_na', [])
    if not isinstance(items, list):
        raise ValueError('approved_na must be a list')
    cases = set()
    for item in items:
        if not isinstance(item, dict) or set(item) != {'record', 'evidence_root', 'evidence'}:
            raise ValueError('invalid approved N/A entry')
        record_path = case_launcher.pinned(item['record'])
        root = Path(item['evidence_root']).resolve(strict=True)
        if not root.is_dir() or record_path.parent != root / 'results':
            raise ValueError('N/A record must belong to its evidence root results directory')
        record = json.loads(record_path.read_text(), object_pairs_hook=manifest_tool.unique_object)
        case = record.get('ID')
        if case not in selected_gpu or case in cases or record_path.name != f'{case}.json':
            raise ValueError('unselected, duplicate or misnamed N/A case')
        approval = record.get('approval')
        if (record.get('result') != 'N-A' or not isinstance(approval, dict)
                or not isinstance(approval.get('scope'), str)
                or not re.search(r'(?<![A-Z0-9-])' + re.escape(case) + r'(?![A-Z0-9-])', approval['scope'])):
            raise ValueError('approved N/A scope must name its selected case')
        validate_results.check_record(record, candidate, root, dt.datetime.now(dt.timezone.utc))
        evidence = item['evidence']
        if not isinstance(evidence, dict) or set(evidence) != set(record['evidence_paths']):
            raise ValueError('N/A evidence pins must cover its approval evidence')
        for name, expected in evidence.items():
            path = (root / name).resolve(strict=True)
            if not path.is_relative_to(root) or digest(path) != expected:
                raise ValueError('N/A approval evidence changed or escaped root')
        cases.add(case)
    return cases


def trigger_deferred_cases(index, required, approved_na, allow_pretrigger):
    """Validate explicit same-run trigger declarations; never imply completion."""
    entries = index.get('deferred_at_trigger', [])
    if not isinstance(entries, list):
        raise ValueError('deferred_at_trigger must be a list')
    if entries and not allow_pretrigger:
        raise ValueError('trigger-deferred cases require explicit pretrigger mode')
    deferred = set()
    for item in entries:
        if not isinstance(item, dict) or set(item) != {'case', 'after'}:
            raise ValueError('invalid trigger-deferred declaration')
        case, after = item['case'], item['after']
        if (not isinstance(case, str) or case not in {'BC-COMP-001', 'BC-OPS-001'}
                or case not in required or case in deferred or case in approved_na):
            raise ValueError('invalid, duplicate or N/A trigger-deferred case')
        if case == 'BC-COMP-001':
            predecessors = {'BC-PERF-001', 'BC-PERF-002'}
            if not predecessors <= required - approved_na:
                raise ValueError('COMP-001 requires both selected performance predecessors')
        else:
            predecessors = required - approved_na - {'BC-OPS-001'}
        if not isinstance(after, list) or after != sorted(predecessors):
            raise ValueError(f'{case} trigger predecessors do not match the selected plan')
        deferred.add(case)
    if allow_pretrigger and not deferred:
        raise ValueError('pretrigger mode requires a trigger-deferred case')
    return deferred


def load_set(selection_path, catalog_path, index_path, allow_pretrigger=False):
    require_python()
    selection = json.loads(Path(selection_path).read_text(), object_pairs_hook=manifest_tool.unique_object)
    if selection.get("catalog_sha256") != digest(catalog_path):
        raise ValueError("selection was frozen against a different catalog")
    with Path(catalog_path).open(newline="") as stream:
        rows = list(csv.DictReader(stream))
    catalog = {row['ID']: row for row in rows}
    if len(catalog) != len(rows) or not catalog:
        raise ValueError('catalog contains duplicate IDs or is empty')
    selected = selection.get('selected')
    if not isinstance(selected, dict) or not selected:
        raise ValueError('selected case set is missing or empty')
    unknown = set(selected) - set(catalog)
    if unknown:
        raise ValueError(f'selection contains unknown cases: {sorted(unknown)}')
    required = {case for case in selected if catalog[case]['Tier'] == 'RC-GPU'}
    if not required:
        raise ValueError('selected GPU case set is empty')
    index = json.loads(Path(index_path).read_text(), object_pairs_hook=manifest_tool.unique_object)
    if index.get("candidate_sha") != selection["candidate_sha"]:
        raise ValueError("selected-set candidate mismatch")
    if digest(selection_path) != index.get("selection_sha256") or digest(catalog_path) != index.get("catalog_sha256"):
        raise ValueError("selection/catalog pin mismatch")
    # A hash only binds the submitted file; it cannot prove cases were not
    # removed before the index was frozen. Recompute the production policy.
    import validate_results
    rows_for_policy = validate_results.load_catalog(Path(catalog_path))
    validate_results.load_selection(Path(selection_path), Path(catalog_path), rows_for_policy,
                                    selection['candidate_sha'])
    approved_na = approved_na_cases(index, required, selection['candidate_sha'])
    deferred = trigger_deferred_cases(index, required, approved_na, allow_pretrigger)
    required -= approved_na | deferred
    entries = []
    seen = set()
    for item in index["manifests"]:
        path = Path(item["path"])
        if digest(path) != item["sha256"]:
            raise ValueError("frozen case manifest hash mismatch")
        manifest = json.loads(path.read_text(), object_pairs_hook=manifest_tool.unique_object)
        if manifest["candidate_sha"] != selection["candidate_sha"] or len(manifest["cases"]) != 1:
            raise ValueError("case manifest candidate/single-case contract mismatch")
        case = next(iter(manifest["cases"]))
        if case not in required or manifest.get('lane') != catalog[case].get('Parallel group'):
            raise ValueError('case manifest is not selected for its catalog lane')
        if case in seen:
            raise ValueError("duplicate case manifest")
        seen.add(case)
        case_launcher.preflight(manifest)
        entries.append((case, path, manifest, item["sha256"]))
    if seen != required:
        raise ValueError(f"manifest set differs from selected GPU cases: missing={sorted(required-seen)} extra={sorted(seen-required)}")
    return entries


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("selection", "catalog", "index"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--preflight", action="store_true")
    parser.add_argument("--pretrigger", action="store_true",
                        help="explicitly preflight or execute the incomplete pre-trigger subset")
    args = parser.parse_args()
    try:
        index_hash = digest(args.index)
        entries = load_set(args.selection, args.catalog, args.index, allow_pretrigger=args.pretrigger)
        index = json.loads(args.index.read_text(), object_pairs_hook=manifest_tool.unique_object)
        selection = json.loads(args.selection.read_text(), object_pairs_hook=manifest_tool.unique_object)
        def check_frozen_set():
            if (digest(args.index) != index_hash or digest(args.selection) != index['selection_sha256']
                    or digest(args.catalog) != index['catalog_sha256']):
                raise ValueError('selected-set index, selection or catalog changed')
            return approved_na_cases(index, set(selection['selected']), selection['candidate_sha'])
        for case in sorted(check_frozen_set()):
            print(f'N-A-RECORD-VALIDATED {case}')
        deferred = index.get('deferred_at_trigger', [])
        for item in deferred:
            print(f'TRIGGER-DEFERRED {item["case"]}: after {", ".join(item["after"])}')
        for case, path, manifest, expected in entries:
            check_frozen_set()
            if args.preflight:
                import generated_inputs
                print(f"{generated_inputs.preflight_label(manifest)} {case}")
                continue
            if digest(path) != expected:
                raise ValueError("case manifest changed after selected-set preflight")
            case_launcher.preflight(manifest)
            case_launcher.run_case(path, manifest, case)
        check_frozen_set()
        if deferred:
            print('PRETRIGGER-INCOMPLETE: freeze and execute trigger-bound cases before final intake')
            return 3
    except (ValueError, KeyError, OSError, RuntimeError, subprocess.SubprocessError) as exc:
        parser.exit(1, f"selected GPU set failed: {exc}\n")


if __name__ == "__main__":
    raise SystemExit(main())
