# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Verify README links to docs against an actual release tarball."""

import html
import posixpath
import re
import sys
import tarfile
from urllib.parse import unquote, urlsplit


def readme_links(text):
    """Collect inline Markdown, reference Markdown, and HTML link targets."""
    definitions = {}
    for match in re.finditer(r"(?m)^\s*\[([^]]+)\]:\s*<?([^\s>]+)>?", text):
        definitions[" ".join(match[1].lower().split())] = match[2]
    links = [m[1] for m in re.finditer(r"!?\[[^]]*\]\(\s*<?([^\s)>]+)>?(?:\s+[^)]*)?\)", text)]
    for match in re.finditer(r"!?\[([^]]+)\]\[([^]]*)\]", text):
        key = " ".join((match[2] or match[1]).lower().split())
        if key not in definitions:
            raise ValueError(f"unresolved README reference: {key}")
        links.append(definitions[key])
    links.extend(m[1] for m in re.finditer(r'''(?i)\bhref\s*=\s*["']([^"']+)["']''', text))
    return links


def check_archive(path):
    with tarfile.open(path, "r:gz") as archive:
        members = archive.getmembers()
        readmes = [m for m in members if len(m.name.split("/")) == 2 and m.name.endswith("/README.md") and m.isfile()]
        if len(readmes) != 1:
            raise ValueError("archive must contain exactly one root README.md")
        root = readmes[0].name.split("/")[0]
        files = {m.name for m in members if m.isfile()}
        content = archive.extractfile(readmes[0]).read().decode("utf-8")
        for target in readme_links(content):
            parsed = urlsplit(html.unescape(target))
            if parsed.scheme or parsed.netloc:
                continue
            link = unquote(parsed.path)
            # README-relative docs links may have redundant ./ segments. Treat
            # parent-relative paths naming docs as errors, never as external.
            if re.match(r"^(?:\.\./)+docs(?:/|$)", link):
                raise ValueError(f"README doc link escapes archive: {target}")
            local = re.sub(r"^(?:\./)+", "", link)
            if not local.startswith("docs/"):
                continue
            normalized = posixpath.normpath(local)
            if normalized != "docs" and not normalized.startswith("docs/"):
                raise ValueError(f"README doc link escapes docs/: {target}")
            if f"{root}/{normalized}" not in files:
                raise ValueError(f"README doc link missing from archive: {target}")


def main():
    if len(sys.argv) != 2:
        raise SystemExit("usage: check_release_readme_links.py ARCHIVE.tar.gz")
    check_archive(sys.argv[1])
    print("README documentation links resolve inside release archive")


if __name__ == "__main__":
    main()
