#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Execution-time provenance shared by case writers and report generators."""
import datetime as dt
import hashlib
import json
import os
import re
from pathlib import Path

PHASES = ("rent", "ssh-ready", "inputs-ready", "server-ready", "first-request", "last-request", "record-validated")


def require_python(version=None):
    import sys
    if (version or sys.version_info) < (3, 12):
        raise RuntimeError("certification harness requires Python 3.12 or newer")


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def phase(root, case, name, timestamp=None):
    if name not in PHASES:
        raise ValueError("unknown certification phase")
    if not re.fullmatch(r"BC-[A-Z]+-[0-9]{3}", case):
        raise ValueError("invalid case ID")
    event = {"case": case, "phase": name, "utc": timestamp or dt.datetime.now(dt.timezone.utc).isoformat()}
    path = Path(root) / "phases.jsonl"
    with path.open("a", encoding="utf-8") as stream:
        import fcntl
        fcntl.flock(stream, fcntl.LOCK_EX)
        stream.write(json.dumps(event, sort_keys=True) + "\n")
        stream.flush()
        os.fsync(stream.fileno())
    return event


def request_clocks(root, paths):
    """Use observed request clocks only, never runner-start or elapsed guesses."""
    starts, ends = [], []
    def clock(value):
        if not isinstance(value, str):
            return None
        try:
            parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
            return parsed.astimezone(dt.timezone.utc) if parsed.tzinfo else None
        except ValueError:
            return None
    for name in paths:
        if not name.endswith((".jsonl", ".ndjson")):
            continue
        with (Path(root) / name).open(encoding="utf-8") as stream:
            for line in stream:
                try:
                    row = json.loads(line)
                except ValueError:
                    continue  # retain interrupted/truncated raw output without inventing clocks
                if not isinstance(row, dict):
                    continue
                if ".request.v" in str(row.get("schema_version", "")):
                    start, end = clock(row.get("started_at")), clock(row.get("completed_at"))
                else:
                    start = clock(row.get("utc")) if row.get("kind") == "send" else None
                    end = clock(row.get("utc")) if row.get("kind") in ("complete", "error") else None
                if start is not None:
                    starts.append(start)
                if end is not None:
                    ends.append(end)
    return {"first_request_utc": min(starts).isoformat() if starts else None,
            "last_request_utc": max(ends).isoformat() if ends else None}


def enrich(root, record):
    root = Path(root).resolve()
    output = {}
    for item in record.get("evidence_paths", []):
        path = (root / item).resolve()
        if not path.is_relative_to(root) or not path.is_file():
            raise ValueError("unsafe or missing evidence path")
        output[item] = digest(path)
    inputs, integrity_errors = {}, []
    def capture(name, path, expected=None):
        try:
            actual = digest(path)
            inputs[name] = actual
            if expected is not None and actual != expected:
                integrity_errors.append(f"{name}: input changed during case execution")
        except (OSError, TypeError, ValueError) as exc:
            inputs[name] = None
            integrity_errors.append(f"{name}: {exc}")
    def load(name, path):
        capture(name, path)
        try:
            value = json.loads(Path(path).read_text())
            if not isinstance(value, dict):
                raise ValueError("input manifest must be an object")
            return value
        except (OSError, ValueError) as exc:
            integrity_errors.append(f"{name}: invalid or unavailable input manifest: {exc}")
            return None
    def nested_pins(value, prefix):
        if isinstance(value, dict):
            if "path" in value and "sha256" in value:
                capture(prefix, value["path"], value["sha256"])
            for key, child in value.items():
                nested_pins(child, prefix + "." + str(key))
        elif isinstance(value, list):
            for index, child in enumerate(value):
                nested_pins(child, prefix + "." + str(index))
    for name in ("CERT_CANDIDATE", "CERT_BASELINE"):
        if os.environ.get(name):
            capture(name, os.environ[name])
    if os.environ.get("CERT_MANIFEST"):
        lane_manifest = load("CERT_MANIFEST", os.environ["CERT_MANIFEST"])
        lane = os.environ.get("CERT_LANE")
        if lane and isinstance(lane_manifest, dict):
            selected = lane_manifest.get("lanes", {}).get(lane)
            if selected is None:
                integrity_errors.append("selected lane is absent from input manifest")
            nested_pins(selected, "lane." + lane)
        else:
            nested_pins(lane_manifest, "lane")
    if os.environ.get("CERT_INPUT_MANIFEST"):
        manifest = load("execution_manifest", os.environ["CERT_INPUT_MANIFEST"])
        if manifest is not None:
            if manifest.get("candidate_sha") != record["candidate_sha"]:
                integrity_errors.append("execution manifest candidate mismatch")
            if os.environ.get("CERT_CASE_ID") != record["ID"]:
                raise ValueError("runner wrote a case outside the selected launcher")
            nested_pins(manifest, "execution")
    if integrity_errors and record["result"] == "PASS":
        raise ValueError("; ".join(integrity_errors))
    if integrity_errors:
        record = {**record, "input_integrity_errors": integrity_errors}
    clocks = request_clocks(root, record.get("evidence_paths", []))
    record = {**record, "request_clocks": clocks}
    phase_root = Path(os.environ.get("CERT_PHASE_ROOT", root))
    for key, name in (("first_request_utc", "first-request"), ("last_request_utc", "last-request")):
        if clocks[key] is not None:
            phase(phase_root, record["ID"], name, clocks[key])
    # Eligibility is only a proposal; reviewed ancestor/diff proof is still required.
    record = {**record, "input_hashes": inputs, "output_hashes": output,
              "carry_over_eligibility": {"eligible": False, "reason": "requires reviewed measurement-code diff and ancestry proof"}}
    record.setdefault("infra_history", [{"type": "completed" if record["result"] == "PASS" else "case_failed",
                                         "reason": record.get("reason"), "start_utc": record.get("start_utc"),
                                         "end_utc": record.get("end_utc")}])
    return record


def report_rows(root):
    """Verify content hashes before deriving rows; never manufacture missing results."""
    rows = []
    for path in sorted((Path(root) / "results").glob("BC-*.json")):
        record = json.loads(path.read_text())
        if not re.fullmatch(r"[a-f0-9]{40}", record.get("candidate_sha", "")) or record.get("result") not in ("PASS", "FAIL"):
            raise ValueError("invalid result record")
        if record.get("ID") != path.stem or not re.fullmatch(r"BC-[A-Z]+-[0-9]{3}", path.stem):
            raise ValueError("record identity mismatch")
        if set(record.get("output_hashes", {})) != set(record.get("evidence_paths", [])):
            raise ValueError("record lacks complete output hashes")
        if record["result"] == "PASS":
            from validate_results import check_record
            check_record(record, record["candidate_sha"], Path(root), dt.datetime.now(dt.timezone.utc))
        for name, expected in record.get("output_hashes", {}).items():
            evidence = (Path(root) / name).resolve()
            if not evidence.is_relative_to(Path(root).resolve()) or digest(evidence) != expected:
                raise ValueError("evidence hash mismatch")
        if "output_hashes" not in record or "infra_history" not in record:
            raise ValueError("record lacks execution-time provenance")
        rows.append(record)
    if not rows:
        raise ValueError("no validated case records")
    return rows


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("--case")
    parser.add_argument("--phase", choices=PHASES)
    args = parser.parse_args()
    require_python()
    if args.phase:
        phase(args.root, args.case, args.phase)
    else:
        print(json.dumps(report_rows(args.root), indent=2, allow_nan=False))
