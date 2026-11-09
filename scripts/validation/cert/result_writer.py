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
from execution_evidence import enrich, require_python

CASE = re.compile(r"BC-[A-Z]+-[0-9]{3}\Z")
SHA = re.compile(r"[0-9a-f]{40}\Z")
SHA256 = re.compile(r"[0-9a-f]{64}\Z")
SMOKE_MARKER = '.stage0-smoke-root.json'
SMOKE_CLASS = 'Stage-0 smoke, not certification'


class NonCertificationEvidence(ValueError):
    """Abort the inventory entirely; do not emit a certification report/sheet."""


def smoke_origin(root):
    """Return the marked root and metadata, or None for certification paths."""
    import manifest_tool
    root = Path(root).resolve()
    found = []
    for directory in (root, *root.parents):
        marker = directory / SMOKE_MARKER
        if marker.is_symlink():
            raise ValueError('smoke root marker cannot be a symlink')
        if marker.exists():
            if not marker.is_file():
                raise ValueError('smoke root marker must be a file')
            data = json.loads(marker.read_text(encoding='utf-8'), object_pairs_hook=manifest_tool.unique_object)
            if (not isinstance(data, dict) or set(data) != {'classification', 'expectations_sha256'}
                    or data['classification'] != SMOKE_CLASS or not isinstance(data['expectations_sha256'], str)
                    or not SHA256.fullmatch(data['expectations_sha256'])):
                raise ValueError('invalid smoke root marker')
            found.append((directory, data))
    if len(found) > 1:
        raise ValueError('nested smoke roots are not permitted')
    return found[0] if found else None


def classify_smoke(root, record):
    """Permanently label records written beneath a marked smoke-only root."""
    origin = smoke_origin(root)
    if origin:
        if 'evidence_class' in record or 'stage0_expectations_sha256' in record:
            raise ValueError('smoke classification is controlled by its root marker')
        return {**record, 'evidence_class': SMOKE_CLASS,
                'stage0_expectations_sha256': origin[1]['expectations_sha256']}
    if record.get('evidence_class') == SMOKE_CLASS:
        raise ValueError('smoke record requires a marked smoke root')
    return record


def require_same_evidence_origin(destination, source, record=None):
    """Never copy evidence across certification/smoke roots or between smoke roots."""
    try:
        dst, src = smoke_origin(destination), smoke_origin(source)
    except (ValueError, OSError) as exc:
        raise NonCertificationEvidence('invalid smoke evidence origin') from exc
    if dst != src:
        raise NonCertificationEvidence('evidence crosses smoke and certification roots')
    if record is not None:
        if not isinstance(record, dict):
            raise NonCertificationEvidence('invalid predecessor record')
        if src is None:
            if record.get('evidence_class', 'certification') != 'certification' or 'stage0_expectations_sha256' in record:
                raise NonCertificationEvidence('non-certification predecessor record')
        elif (record.get('evidence_class') != SMOKE_CLASS
              or record.get('stage0_expectations_sha256') != src[1]['expectations_sha256']):
            raise NonCertificationEvidence('smoke predecessor classification mismatch')


def require_input_origin(destination, source):
    """Frozen cert inputs may feed smoke; smoke inputs cannot feed certification."""
    try:
        dst, src = smoke_origin(destination), smoke_origin(source)
    except (ValueError, OSError) as exc:
        raise NonCertificationEvidence('invalid input evidence origin') from exc
    if src is not None and dst != src:
        raise NonCertificationEvidence('input crosses smoke evidence roots')


def reject_smoke_root(directory):
    try:
        marked = classify_smoke(Path(directory).resolve(), {})
    except (ValueError, OSError) as exc:
        raise NonCertificationEvidence('invalid smoke root marker') from exc
    if marked:
        raise NonCertificationEvidence('marked smoke root cannot enter certification intake or history')


def write_record(root, record):
    require_python()
    root = Path(root).resolve()
    record = classify_smoke(root, record)
    # The lane-level attempt number is supplied by the operator for an
    # unchanged rerun; never overwrite an earlier attempt directory.
    attempt = os.environ.get("CERT_ATTEMPT")
    if attempt is not None:
        if not re.fullmatch(r"[1-9][0-9]*", attempt):
            raise ValueError("CERT_ATTEMPT must be a positive integer")
        record = {**record, "attempt": int(attempt)}
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
    record = enrich(root, record)
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
