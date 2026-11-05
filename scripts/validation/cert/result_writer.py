#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Write one atomic case record with provenance; never infer PASS from exit code."""
import argparse
import json
import os
import re
import tempfile
from pathlib import Path

CASE = re.compile(r"BC-[A-Z]+-[0-9]{3}\Z")
SHA = re.compile(r"[0-9a-f]{40}\Z")


def write_record(root, record):
    root = Path(root).resolve()
    case = record.get("ID", "")
    if not CASE.fullmatch(case) or record.get("result") not in ("PASS", "FAIL"):
        raise ValueError("invalid case ID or result")
    if not SHA.fullmatch(str(record.get("candidate_sha", ""))):
        raise ValueError("invalid candidate commit SHA")
    if record["result"] == "PASS":
        for key in ("observed_values", "thresholds", "versions"):
            if not isinstance(record.get(key), dict) or not record[key]:
                raise ValueError(f"PASS requires nonempty {key}")
        for key in ("owner", "evidence_link", "start_utc", "end_utc", "gpu_class", "driver"):
            if not isinstance(record.get(key), str) or not record[key].strip():
                raise ValueError(f"PASS requires {key}")
        if not isinstance(record.get("attempt"), int) or isinstance(record["attempt"], bool) or record["attempt"] < 1:
            raise ValueError("PASS requires a positive attempt number")
        if not record.get("evidence_paths"):
            raise ValueError("PASS requires evidence")
    for item in record.get("evidence_paths", []):
        if not isinstance(item, str) or not item:
            raise ValueError("invalid evidence path")
        evidence = (root / item).resolve()
        if not evidence.is_relative_to(root) or not evidence.is_file():
            raise ValueError("evidence path does not resolve inside the run directory")
    directory = root / "results"
    directory.mkdir(exist_ok=True)
    target = directory / (case + ".json")
    if target.exists():
        raise FileExistsError("refusing to overwrite an existing case result")
    content = json.dumps(record, indent=2, sort_keys=True, allow_nan=False) + "\n"
    fd, temporary = tempfile.mkstemp(prefix=".result-", dir=directory)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as stream:
            stream.write(content)
            stream.flush()
            os.fsync(stream.fileno())
        # Hard-link gives atomic publication without silently replacing existing results.
        os.link(temporary, target)
    finally:
        os.unlink(temporary)
    return target


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--record", type=Path, required=True)
    args = parser.parse_args()
    try:
        record = json.loads(args.record.read_text(encoding="utf-8"))
        write_record(args.out, record)
        return 0
    except (OSError, ValueError, TypeError) as exc:
        parser.exit(1, f"result write failed: {exc}\n")


if __name__ == "__main__":
    raise SystemExit(main())
