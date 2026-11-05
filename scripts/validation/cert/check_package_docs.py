#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Compare archive docs byte-for-byte with tracked docs at a git revision."""
import argparse
import subprocess
import sys
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "scripts"))
from check_release_readme_links import check_archive  # noqa: E402

TARGETS = ("x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu",
           "x86_64-apple-darwin", "aarch64-apple-darwin")


def git(*args):
    return subprocess.check_output(["git", "-C", str(ROOT), *args])


def tracked_docs(revision):
    paths = git("ls-tree", "-r", "-z", "--name-only", revision, "--", "docs").decode().split("\0")
    paths = [path for path in paths if path]
    if not paths:
        raise ValueError("no tracked documentation in revision")
    return {path: git("show", f"{revision}:{path}") for path in paths}


def check_docs(path, expected):
    with tarfile.open(path, "r:gz") as archive:
        members = archive.getmembers()
        roots = {member.name.split("/")[0] for member in members if member.name}
        if len(roots) != 1:
            raise ValueError("archive must have exactly one root")
        root = roots.pop()
        entries = {}
        for member in members:
            prefix = root + "/docs/"
            if member.name.startswith(prefix) and not member.isdir():
                if not member.isfile():
                    raise ValueError(f"non-regular documentation entry: {member.name}")
                relative = member.name[len(root) + 1:]
                if relative in entries:
                    raise ValueError(f"duplicate documentation entry: {relative}")
                entries[relative] = archive.extractfile(member).read()
        if set(entries) != set(expected):
            raise ValueError(f"docs file set mismatch: missing={sorted(set(expected) - set(entries))}; extra={sorted(set(entries) - set(expected))}")
        for name, data in entries.items():
            if data != expected[name]:
                raise ValueError(f"docs content mismatch: {name}")
    check_archive(str(path))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--revision", default="HEAD")
    parser.add_argument("--archive", type=Path, action="append", default=[])
    parser.add_argument("--dry-run", action="store_true", help="exercise tracked docs packaging for each target; does not build binaries")
    args = parser.parse_args()
    if args.dry_run == bool(args.archive):
        parser.error("choose --dry-run or one or more --archive")
    try:
        docs = tracked_docs(args.revision)
        if args.dry_run:
            import tempfile
            with tempfile.TemporaryDirectory() as directory:
                for target in TARGETS:
                    # Mirrors the release packaging's git archive of docs; no platform binaries built.
                    data = git("archive", "--format=tar.gz", f"--prefix=cert-{target}/", args.revision, "docs", "README.md")
                    path = Path(directory) / f"{target}.tar.gz"
                    path.write_bytes(data)
                    check_docs(path, docs)
                    print(f"{target}: tracked documentation and README links PASS")
        else:
            for path in args.archive:
                check_docs(path, docs)
                print(f"{path.name}: tracked documentation and README links PASS")
        return 0
    except (OSError, ValueError, subprocess.CalledProcessError, tarfile.TarError) as exc:
        print(f"documentation archive check failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
