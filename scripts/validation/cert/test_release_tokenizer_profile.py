# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline policy checks; not evidence of a built release artifact."""
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]


class ReleaseTokenizerProfileTests(unittest.TestCase):
    def test_default_install_and_tokenizer_dependency_are_explicit(self):
        manifest = tomllib.loads((ROOT / 'Cargo.toml').read_text())
        self.assertEqual(manifest['features']['default'], ['tokenizer'])
        self.assertEqual(manifest['features']['tokenizer'], ['dep:tokenizers', 'dep:minijinja'])
        self.assertTrue(manifest['dependencies']['tokenizers']['optional'])
        self.assertTrue(manifest['dependencies']['minijinja']['optional'])

    def test_release_and_pretag_use_same_explicit_shipped_features(self):
        release = (ROOT / '.github/workflows/release.yml').read_text()
        pretag = (ROOT / '.github/workflows/pretag-certification.yml').read_text()
        self.assertIn('cargo zigbuild --locked --release --features otlp,tokenizer\n', release)
        self.assertIn('cargo publish --dry-run --locked --features otlp,tokenizer', release)
        self.assertIn('cargo publish --locked --features otlp,tokenizer', release)
        self.assertIn('cargo zigbuild --locked --release --features otlp,tokenizer --target "${{ matrix.zig_target }}" --bins', pretag)
        self.assertIn('cargo publish --dry-run --locked --features otlp,tokenizer', pretag)


if __name__ == '__main__':
    unittest.main()
