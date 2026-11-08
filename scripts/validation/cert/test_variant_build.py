# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline policy checks for the shipped-only GPU010 build profile."""
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]


class ShippedProfileTests(unittest.TestCase):
    def test_pretag_has_shipped_profile_and_no_separate_variant(self):
        workflow = (ROOT / '.github/workflows/pretag-certification.yml').read_text()
        self.assertIn('cargo zigbuild --locked --release --features otlp,tokenizer --target "${{ matrix.zig_target }}" --bins', workflow)
        self.assertIn('cargo publish --dry-run --locked --features otlp,tokenizer', workflow)
        self.assertNotIn('  tokenizer-variant:', workflow)
        self.assertNotIn('pretag-tokenizer-x86_64-unknown-linux-gnu', workflow)

    def test_freezer_has_no_variant_cli_or_download(self):
        source = (ROOT / 'scripts/validation/cert/freeze_builds.py').read_text()
        self.assertNotIn('--tokenizer-variant', source)
        self.assertNotIn('variant=True', source)


if __name__ == '__main__':
    unittest.main()
