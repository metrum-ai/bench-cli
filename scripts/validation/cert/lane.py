#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Certification lane plan and explicit fail-closed implementation status."""
import argparse
import csv
import datetime as dt
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
CATALOG = ROOT / "docs/RELEASE_CERTIFICATION_CATALOG.csv"
LANES = ("P1", "P2", "L1", "L2", "L3", "L4", "R", "D", "V", "W", "G1", "G2", "G3", "G4", "G5", "G6", "G7")


def cases(lane):
    with CATALOG.open(newline="", encoding="utf-8") as stream:
        return [r for r in csv.DictReader(stream) if r["Parallel group"] == lane and r["Tier"] == "RC-GPU"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("lane", choices=LANES)
    parser.add_argument("--plan", action="store_true")
    parser.add_argument("--candidate", type=Path)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    selected = cases(args.lane)
    if not selected:
        parser.error("lane has no catalog cases")
    if args.plan:
        print(f"Lane {args.lane}: PLAN ONLY (no tests run; implementation status varies by lane)")
        for case in selected:
            print(f"{case['ID']}: {case['Command or procedure']} -> {case['Pass criteria']}")
        return 0
    if not all((args.candidate, args.baseline, args.manifest, args.out)):
        parser.error("--candidate, --baseline, --manifest and --out are required")
    if not all(p.is_file() for p in (args.candidate, args.baseline, args.manifest)):
        parser.error("candidate, baseline and manifest must be existing files")
    # Never start a server or report a pass until the real lane implementation exists.
    args.out.mkdir(parents=True, exist_ok=False)
    result_dir = args.out / "results"
    result_dir.mkdir()
    now = dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z")
    for case in selected:
        record = {"ID": case["ID"], "result": "FAIL", "reason": "lane not implemented; no test executed",
                  "observed_values": {}, "thresholds": {}, "evidence_paths": [], "attempt": 1,
                  "start_utc": now, "end_utc": now, "gpu_class": None, "driver": None, "versions": {}}
        (result_dir / (case["ID"] + ".json")).write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
    print(f"Lane {args.lane} is not implemented; all {len(selected)} cases FAIL.")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
