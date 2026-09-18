#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Package the Bench CLI Docusaurus build into a versioned tarball for Restic upload.

Writes the docs.metrum.ai standard manifest schema (generated_at / latest /
product / versions[] / aliases[]). The consumer is docs-shell/registry.json
in metrum-internal-infra-admin.
"""

import argparse
import hashlib
import json
import shutil
import sys
import tarfile
import time
from pathlib import Path

SITE_ROOT = "metrum-ai-bench-cli"
PRODUCT_NAME = "Metrum AI Bench CLI"


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def build_manifest(release_versions: list[str], latest_version: str, site_root: str) -> dict:
    version_entries = [
        {
            "version": v,
            "label": v,
            "path": f"/{site_root}/{v}/",
            "is_latest": v == latest_version,
        }
        for v in sorted(set(release_versions), reverse=True)
    ]
    return {
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "latest": latest_version,
        "product": PRODUCT_NAME,
        "versions": version_entries,
        "aliases": [
            {
                "label": "latest",
                "path": f"/{site_root}/latest/",
                "version": latest_version,
            }
        ],
    }


def main() -> None:
    parser = argparse.ArgumentParser(description="Package Bench CLI docs for release.")
    parser.add_argument("--version", required=True, help="Docs version, e.g. v1.0.0")
    parser.add_argument("--latest-version", required=True, help="Version to alias as latest")
    parser.add_argument("--release-versions", required=True, help="Space-separated list of released versions")
    parser.add_argument("--site-root", default=SITE_ROOT, help="Site root slug")
    parser.add_argument("--dist-dir", default="dist", help="Output directory")
    parser.add_argument("--build-dir", default="build", help="Docusaurus build output")
    parser.add_argument(
        "--latest-build-dir",
        default="",
        help="Separate build for /latest/ (built with baseUrl=/latest/). Falls back to --build-dir if absent.",
    )
    args = parser.parse_args()

    version = args.version
    site_root = args.site_root
    dist_dir = Path(args.dist_dir)
    build_dir = Path(args.build_dir)
    latest_build_dir = Path(args.latest_build_dir) if args.latest_build_dir else None

    if not build_dir.exists():
        print(f"ERROR: build dir {build_dir} does not exist. Run 'make build' first.", file=sys.stderr)
        sys.exit(1)

    if latest_build_dir and not latest_build_dir.exists():
        print(f"WARNING: latest-build-dir {latest_build_dir} not found, falling back to {build_dir}", file=sys.stderr)
        latest_build_dir = None

    dist_dir.mkdir(parents=True, exist_ok=True)

    staging = dist_dir / "staging"
    if staging.exists():
        shutil.rmtree(staging)
    staging.mkdir()

    version_dest = staging / site_root / version
    shutil.copytree(build_dir, version_dest)

    # Only ship a latest/ tree when this build is actually promoting to
    # latest (latest_build_dir given and present). A non-promoted publish
    # (e.g. a -rc. tag) must never carry a latest/ tree in its archive: the
    # restore script uses that tree's presence to decide whether to touch
    # the live /latest/ alias at all, so a stray latest/ here would clobber
    # the real latest release with this non-promoted build's own content.
    if latest_build_dir:
        shutil.copytree(latest_build_dir, staging / site_root / "latest")

    release_versions = args.release_versions.split()
    if version not in release_versions:
        release_versions.append(version)
    manifest = build_manifest(release_versions, args.latest_version, site_root)

    versions_json = staging / site_root / "versions.json"
    versions_json.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    archive_name = f"{site_root}-docs-{version}.tar.gz"
    archive_path = dist_dir / archive_name
    with tarfile.open(archive_path, "w:gz") as tar:
        tar.add(staging / site_root, arcname=site_root)

    checksum = sha256_file(archive_path)
    checksum_path = dist_dir / f"{archive_name}.sha256"
    checksum_path.write_text(f"{checksum}  {archive_name}\n")

    versions_manifest = dist_dir / f"{site_root}-docs-versions.json"
    versions_manifest.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    shutil.rmtree(staging)

    print(f"Packaged: {archive_path}")
    print(f"Checksum: {checksum_path}")
    print(f"Versions: {versions_manifest}")
    print(f"SHA256:   {checksum}")


if __name__ == "__main__":
    main()
