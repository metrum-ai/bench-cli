#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Freeze one selected GPU case and every declared local input before launch."""
import argparse
import csv
import json
import subprocess
from pathlib import Path
import case_launcher
import build_roles
import manifest_tool
from execution_evidence import digest, require_python


def pin(path):
    path = Path(path).resolve(strict=True)
    if not path.is_file():
        raise ValueError("frozen inputs must be regular files")
    return {"path": str(path), "sha256": digest(path)}


def check_case_tokenizer(case, config, path):
    item = config.get("tokenizer")
    if case == "BC-GPU-010" and (not isinstance(item, dict) or item.get("sha256") != digest(path)):
        raise ValueError("BC-GPU-010 must probe the same tokenizer bytes as its runner")


def harness_sources(root):
    """Pin interpreted and compiled harness inputs, including Cargo's alias."""
    root = Path(root).resolve()
    repo = root.parents[2]
    sources = {path.resolve() for path in root.iterdir() if path.suffix in ('.py', '.sh', '.json')}
    sources.update(path.resolve() for path in (repo / 'xtask/src').rglob('*.rs'))
    # count_points is an external #[path] module compiled into xtask.
    for relative in ('Cargo.toml', 'Cargo.lock', 'xtask/Cargo.toml', 'xtask/src/main.rs',
                     'src/count_points.rs', '.cargo/config.toml'):
        sources.add((repo / relative).resolve(strict=True))
    for relative in ('xtask/Cargo.lock', 'xtask/build.rs', 'build.rs', '.cargo/config',
                     'rust-toolchain', 'rust-toolchain.toml'):
        path = repo / relative
        if path.is_file():
            sources.add(path.resolve())
    return sources


def freeze(args):
    require_python()
    selection = json.loads(args.selection.read_text())
    if selection.get("catalog_sha256") != digest(args.catalog):
        raise ValueError("selection was frozen against a different catalog")
    if args.case not in selection["selected"]:
        raise ValueError("case is not selected")
    with args.catalog.open(newline="") as stream:
        catalog = {row["ID"]: row for row in csv.DictReader(stream)}
    row = catalog[args.case]
    if row["Tier"] != "RC-GPU":
        raise ValueError("per-case GPU launcher requires an RC-GPU catalog case")
    lane_manifest = json.loads(args.lane_manifest.read_text())
    if lane_manifest["candidate_sha"] != selection["candidate_sha"]:
        raise ValueError("lane manifest and selection candidate mismatch")
    if digest(args.candidate) != lane_manifest["candidate_binary_sha256"] or digest(args.baseline) != lane_manifest["baseline_binary_sha256"]:
        raise ValueError("lane manifest binary hash mismatch")
    measured, validation = build_roles.load_receipt(args.build_receipt, selection['candidate_sha'])
    matching = [item for item in measured if item['sha256'] == digest(args.candidate)
                and Path(item['path']).resolve() == args.candidate.resolve()]
    if len(matching) != 1:
        raise ValueError('candidate must be a measured artifact from the build receipt')
    build_roles.check_measured_config(lane_manifest, measured, validation)
    variant = build_roles.load_variant(args.build_receipt, selection['candidate_sha']) if args.case == 'BC-GPU-010' else None
    import generated_inputs
    recipe_file = getattr(args, 'generated_recipes', None)
    generated = []
    if recipe_file is not None:
        values = json.loads(recipe_file.read_text(), object_pairs_hook=manifest_tool.unique_object)['recipes']
        if not isinstance(values, list) or not values:
            raise ValueError('generated recipe file must declare a nonempty recipe list')
        generated = [{'recipe': value, 'structural': generated_inputs.validate(
            value, args.build_receipt, selection['candidate_sha'])} for value in values]
        if len({item['recipe']['id'] for item in generated}) != len(generated):
            raise ValueError('duplicate generated recipe id')
    lane = row["Parallel group"]
    if generated and lane == 'CLEANUP':
        raise ValueError('cleanup cases cannot generate measured inputs')
    if lane != "CLEANUP":
        structural = {'structural_recipe_ids': {item['recipe']['id'] for item in generated}} if generated else {}
        _, config = manifest_tool.load_manifest(args.lane_manifest, lane, args.candidate, args.baseline, "0", **structural)
        if args.tokenizer_revision != config["revision"]:
            raise ValueError("tokenizer revision must match the frozen model revision")
        check_case_tokenizer(args.case, config, args.tokenizer)
    root = Path(__file__).parent
    if args.command_json:
        if args.case not in ('BC-COMP-001', 'BC-OPS-001', 'BC-INTEG-001'):
            raise ValueError('custom commands are restricted to barrier/cleanup/signoff cases')
        command = json.loads(args.command_json.read_text())
    else:
        if args.case in ("BC-COMP-001", "BC-OPS-001", "BC-INTEG-001"):
            raise ValueError("barrier/cleanup/external-signoff cases require an explicit frozen command")
        lane_name = "comp002" if args.case == "BC-COMP-002" else row['Parallel group'].lower()
        command = [str((root / "case_entry.sh").resolve()), "--case", args.case, "--lane", lane_name,
                   "--candidate", str(args.candidate.resolve()),
                   "--baseline", str(args.baseline.resolve()), "--manifest", str(args.lane_manifest.resolve()),
                   "--out", str(args.run_out.resolve())]
    inputs = {Path(path).resolve() for path in args.input}
    inputs.update({args.selection.resolve(), args.catalog.resolve(), args.lane_manifest.resolve(), args.baseline.resolve(),
                   args.build_receipt.resolve(), Path(validation['path']).resolve()})
    inputs.update(Path(item['path']).resolve() for item in measured)
    inputs.add(args.build_receipt.resolve().parent / 'validation-only.log')
    origin = measured[0]['origin']
    inputs.update(Path(origin[key]['path']).resolve() for key in
                  ('run', 'artifact_metadata', 'artifact_listing', 'archive', 'checksum', 'download_log'))
    if variant is not None:
        import gpu010_builds
        inputs.add(Path(variant['path']).resolve())
        inputs.update(Path(variant['origin'][key]['path']).resolve() for key in gpu010_builds.ORIGIN_PINS)
    def collect_pins(value):
        if isinstance(value, dict):
            if "path" in value and "sha256" in value:
                item = pin(value["path"])
                if item["sha256"] != value["sha256"]:
                    raise ValueError("lane input pin mismatch")
                inputs.add(Path(item["path"]))
            for child in value.values():
                collect_pins(child)
        elif isinstance(value, list):
            for child in value:
                collect_pins(child)
    collect_pins(lane_manifest)
    if generated:
        inputs.add(recipe_file.resolve())
        collect_pins(generated)
    # Freeze transitive harness sources, not only the top-level shell wrapper.
    inputs.update(harness_sources(root))
    if args.command_json:
        inputs.add(args.command_json.resolve())
    manifest = {"$comment": "Copyright (c) 2026 Metrum AI, Inc. SPDX-License-Identifier: Apache-2.0",
                "candidate_sha": selection["candidate_sha"], "candidate": matching[0],
                "tokenizer_validation": validation, "measured_binaries": measured,
                "build_receipt": pin(args.build_receipt), "lane_manifest": pin(args.lane_manifest),
                "sut": pin(args.sut),
                "tokenizer": {**pin(args.tokenizer), "revision": args.tokenizer_revision}, "all_smi": pin(args.all_smi),
                "inputs": [pin(path) for path in sorted(inputs)],
                "cases": {args.case: {"launcher": pin(command[0]), "command": command, "out": str(args.run_out.resolve())}}}
    if variant is not None:
        manifest['feature_variant_tokenizer'] = variant
    if generated:
        manifest.update(generated_inputs=generated, lane=lane, baseline=pin(args.baseline))
    if lane == 'L1':
        import derived_inputs
        manifest.update(lane=lane, baseline=pin(args.baseline))
        manifest['derived_inputs'] = derived_inputs.declaration(manifest)
    case_launcher.preflight(manifest)
    with args.out.open("x", encoding="utf-8") as stream:
        json.dump(manifest, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("selection", "catalog", "lane-manifest", "candidate", "baseline", "sut", "tokenizer", "all-smi", "build-receipt", "out", "run-out"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--case", required=True)
    parser.add_argument("--tokenizer-revision", required=True)
    parser.add_argument("--input", type=Path, action="append", default=[])
    parser.add_argument("--command-json", type=Path)
    parser.add_argument('--generated-recipes', type=Path)
    args = parser.parse_args()
    try:
        freeze(args)
    except (ValueError, KeyError, OSError, RuntimeError, subprocess.SubprocessError) as exc:
        parser.exit(1, f"case freeze failed: {exc}\n")
    import generated_inputs
    frozen = json.loads(args.out.read_text())
    print(f"{generated_inputs.preflight_label(frozen)} {args.case}; frozen input manifest: {args.out}")


if __name__ == "__main__":
    main()
