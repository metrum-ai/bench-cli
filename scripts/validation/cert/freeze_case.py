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
import manifest_tool
from execution_evidence import digest, require_python


def pin(path):
    path = Path(path).resolve(strict=True)
    if not path.is_file():
        raise ValueError("frozen inputs must be regular files")
    return {"path": str(path), "sha256": digest(path)}


def freeze(args):
    require_python()
    selection = json.loads(args.selection.read_text())
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
    lane = row["Parallel group"]
    if lane != "CLEANUP":
        manifest_tool.load_manifest(args.lane_manifest, lane, args.candidate, args.baseline, "0")
    root = Path(__file__).parent
    if args.command_json:
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
    inputs.update({args.selection.resolve(), args.catalog.resolve(), args.lane_manifest.resolve(), args.baseline.resolve()})
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
    # Freeze transitive harness sources, not only the top-level shell wrapper.
    inputs.update(path.resolve() for path in root.iterdir() if path.suffix in (".py", ".sh", ".json"))
    if args.command_json:
        inputs.add(args.command_json.resolve())
    manifest = {"$comment": "Copyright (c) 2026 Metrum AI, Inc. SPDX-License-Identifier: Apache-2.0",
                "candidate_sha": selection["candidate_sha"], "candidate": pin(args.candidate), "sut": pin(args.sut),
                "tokenizer": {**pin(args.tokenizer), "revision": args.tokenizer_revision}, "all_smi": pin(args.all_smi),
                "inputs": [pin(path) for path in sorted(inputs)],
                "cases": {args.case: {"launcher": pin(command[0]), "command": command, "out": str(args.run_out.resolve())}}}
    case_launcher.preflight(manifest)
    with args.out.open("x", encoding="utf-8") as stream:
        json.dump(manifest, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("selection", "catalog", "lane-manifest", "candidate", "baseline", "sut", "tokenizer", "all-smi", "out", "run-out"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--case", required=True)
    parser.add_argument("--tokenizer-revision", required=True)
    parser.add_argument("--input", type=Path, action="append", default=[])
    parser.add_argument("--command-json", type=Path)
    args = parser.parse_args()
    try:
        freeze(args)
    except (ValueError, KeyError, OSError, RuntimeError, subprocess.SubprocessError) as exc:
        parser.exit(1, f"case freeze failed: {exc}\n")
    print(f"PREFLIGHT-OK {args.case}; frozen input manifest: {args.out}")


if __name__ == "__main__":
    main()
