# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0

import io
import tarfile
import tempfile
import unittest
from pathlib import Path

from check_release_readme_links import check_archive


class ArchiveLinkTests(unittest.TestCase):
    def make_archive(self, directory, readme, docs):
        path = Path(directory) / "release.tar.gz"
        with tarfile.open(path, "w:gz") as archive:
            for name, content in {"README.md": readme, **docs}.items():
                data = content.encode()
                info = tarfile.TarInfo(f"release/{name}")
                info.size = len(data)
                archive.addfile(info, io.BytesIO(data))
        return path

    def test_nested_fragment_query_reference_and_html(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self.make_archive(directory, '[a](./docs/nested/a.md?q=x#h) [b][label] <a href="docs/a.md#x">x</a>\n\n[label]: docs/a.md\n', {"docs/nested/a.md": "ok", "docs/a.md": "ok"})
            check_archive(path)

    def test_missing_doc_and_escape_fail(self):
        with tempfile.TemporaryDirectory() as directory:
            for link in ("docs/missing.md#x", "./docs/missing.md", "docs/../README.md", "../docs/a.md"):
                path = self.make_archive(directory, f"[a]({link})", {})
                with self.assertRaises(ValueError):
                    check_archive(path)


if __name__ == "__main__":
    unittest.main()
