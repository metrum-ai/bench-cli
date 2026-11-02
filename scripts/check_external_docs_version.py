#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Keep external release notes and current-release labels aligned with Cargo."""
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RELEASE_HEADING = re.compile(r"^## Metrum AI Bench CLI (\d+\.\d+\.\d+(?:[-+][\w.-]+)?)\s*$", re.MULTILINE)
CURRENT_RELEASE = re.compile(
    r"\bcurrent\s+release\s*(?::|is\b)\s*(?:\*{1,2}|`|<strong>|\[)*\s*v?(\d+\.\d+\.\d+(?:[-+][\w.-]+)?)",
    re.IGNORECASE,
)


def check(root: Path) -> list[str]:
    with (root / "Cargo.toml").open("rb") as f:
        version = tomllib.load(f)["package"]["version"]
    content = root / "external-docs" / "content"
    notes = content / "docs" / "release-notes.mdx"
    errors = []
    headings = RELEASE_HEADING.findall(notes.read_text(encoding="utf-8"))
    if version not in headings:
        errors.append(f"{notes}: missing release section for Cargo version {version}")
    for path in sorted(content.rglob("*.mdx")):
        for match in CURRENT_RELEASE.finditer(path.read_text(encoding="utf-8")):
            if match.group(1) != version:
                errors.append(f"{path}: current release {match.group(1)} does not match Cargo {version}")
    return errors


def main() -> int:
    try:
        errors = check(ROOT)
    except (OSError, KeyError, tomllib.TOMLDecodeError) as exc:
        print(f"external docs version check failed: {exc}", file=sys.stderr)
        return 1
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print("external docs version check: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
