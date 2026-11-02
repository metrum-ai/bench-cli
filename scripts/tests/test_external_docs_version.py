# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Regression checks for external documentation release-version drift."""
import tempfile
import unittest
from pathlib import Path
from scripts.check_external_docs_version import check


class DocsVersionTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root / 'Cargo.toml').write_text('[package]\nname = "example"\nversion = "1.7.1"\n')
        self.content = self.root / 'external-docs' / 'content'
        (self.content / 'docs').mkdir(parents=True)
        (self.content / 'docs' / 'release-notes.mdx').write_text('## Metrum AI Bench CLI 1.7.1\n')
        (self.content / 'intro.mdx').write_text('Current release: <CurrentRelease />.\n')

    def test_dynamic_version_and_matching_heading_pass(self):
        self.assertEqual(check(self.root), [])

    def test_missing_or_partial_release_heading_fails(self):
        (self.content / 'docs' / 'release-notes.mdx').write_text('## Metrum AI Bench CLI 1.7.10\n')
        self.assertTrue(any('missing release section' in e for e in check(self.root)))

    def test_old_current_release_in_nested_page_fails(self):
        (self.content / 'docs' / 'other.mdx').write_text('Current release: **v1.5.5**.\n')
        self.assertTrue(any('1.5.5' in e for e in check(self.root)))

    def test_historical_version_and_matching_literal_are_allowed(self):
        (self.content / 'intro.mdx').write_text('Current release: **1.7.1**.\nRemoved in 1.3.0.\n')
        self.assertEqual(check(self.root), [])

    def test_prose_and_linked_version_are_checked(self):
        (self.content / 'intro.mdx').write_text('The current release is [v1.5.5](../release).\n')
        self.assertTrue(check(self.root))

    def test_html_version_is_checked(self):
        (self.content / 'intro.mdx').write_text('Current release: <strong>1.5.5</strong>.\n')
        self.assertTrue(check(self.root))


if __name__ == '__main__':
    unittest.main()
