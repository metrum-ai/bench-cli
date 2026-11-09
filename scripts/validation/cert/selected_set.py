#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""One offline-preflight/launch entry for a complete selected GPU manifest set."""
import argparse
import csv
import json
import subprocess
from pathlib import Path
import case_launcher
import manifest_tool
from execution_evidence import digest, require_python


def load_set(selection_path, catalog_path, index_path):
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
    args = parser.parse_args()
    try:
        entries = load_set(args.selection, args.catalog, args.index)
        for case, path, manifest, expected in entries:
            if args.preflight:
                import generated_inputs
                print(f"{generated_inputs.preflight_label(manifest)} {case}")
                continue
            if digest(path) != expected:
                raise ValueError("case manifest changed after selected-set preflight")
            case_launcher.preflight(manifest)
            case_launcher.run_case(path, manifest, case)
    except (ValueError, KeyError, OSError, RuntimeError, subprocess.SubprocessError) as exc:
        parser.exit(1, f"selected GPU set failed: {exc}\n")


if __name__ == "__main__":
    main()
