#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Reject Cargo.lock packages whose declared rust-version exceeds the CI MSRV.

Cargo may report a cached check as fresh without rebuilding every dependency.
This preflight checks rust-version metadata for the complete locked package set,
independently of target artifacts, so a cache hit cannot mask an MSRV mismatch.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import tomllib
from pathlib import Path
from typing import Any


def version_tuple(value: str) -> tuple[int, int, int]:
    match = re.fullmatch(r"(\d+)\.(\d+)(?:\.(\d+))?", value)
    if not match:
        raise ValueError(f"unsupported rust-version format: {value!r}")
    major, minor, patch = match.groups()
    return int(major), int(minor), int(patch or 0)


def package_key(package: dict[str, Any]) -> tuple[str, str, str | None]:
    return package["name"], package["version"], package.get("source")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--msrv", required=True, help="minimum Rust version for this job")
    parser.add_argument("--lock-file", type=Path, default=Path("Cargo.lock"))
    parser.add_argument("metadata", type=Path, help="cargo metadata JSON output")
    args = parser.parse_args()

    try:
        supported = version_tuple(args.msrv)
        metadata = json.loads(args.metadata.read_text(encoding="utf-8"))
        lock = tomllib.loads(args.lock_file.read_text(encoding="utf-8"))
        lock_packages = {package_key(package) for package in lock["package"]}
        metadata_packages = metadata["packages"]
        metadata_keys = {package_key(package) for package in metadata_packages}
    except (OSError, KeyError, json.JSONDecodeError, tomllib.TOMLDecodeError, ValueError) as exc:
        print(f"locked MSRV check: cannot read/parse inputs: {exc}", file=sys.stderr)
        return 2

    if lock_packages != metadata_keys:
        missing = sorted(lock_packages - metadata_keys)
        unexpected = sorted(metadata_keys - lock_packages)
        print("locked MSRV check: cargo metadata package set does not match Cargo.lock", file=sys.stderr)
        if missing:
            print(f"missing from metadata: {missing}", file=sys.stderr)
        if unexpected:
            print(f"not in lock file: {unexpected}", file=sys.stderr)
        return 2

    incompatible: list[tuple[tuple[int, int, int], str, str, str]] = []
    for package in metadata_packages:
        required = package.get("rust_version")
        if required is None:
            continue
        try:
            required_tuple = version_tuple(required)
        except ValueError as exc:
            print(f"locked MSRV check: {package['name']} {package['version']}: {exc}", file=sys.stderr)
            return 2
        if required_tuple > supported:
            incompatible.append((required_tuple, package["name"], package["version"], required))

    if incompatible:
        print(f"locked MSRV check failed: locked dependencies exceed Rust {args.msrv}:", file=sys.stderr)
        for _, name, version, required in sorted(incompatible):
            print(f"  {name} {version} requires Rust {required}", file=sys.stderr)
        return 1

    print(f"locked MSRV check passed: all {len(lock_packages)} locked packages support Rust {args.msrv}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
