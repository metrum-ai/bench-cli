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
from execution_evidence import digest, require_python


def load_set(selection_path, catalog_path, index_path):
    require_python()
    selection = json.loads(Path(selection_path).read_text())
    if selection.get("catalog_sha256") != digest(catalog_path):
        raise ValueError("selection was frozen against a different catalog")
    with Path(catalog_path).open(newline="") as stream:
        gpu = {row["ID"] for row in csv.DictReader(stream) if row["Tier"] == "RC-GPU"}
    required = set(selection["selected"]) & gpu
    index = json.loads(Path(index_path).read_text())
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
        manifest = json.loads(path.read_text())
        if manifest["candidate_sha"] != selection["candidate_sha"] or len(manifest["cases"]) != 1:
            raise ValueError("case manifest candidate/single-case contract mismatch")
        case = next(iter(manifest["cases"]))
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
