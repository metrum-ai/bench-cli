# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Tests for the all-smi release-asset provenance check."""
import hashlib
import io
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import perf_all_smi


def make_asset(root, binary_bytes):
    asset = root / 'all-smi-linux-x86_64.tar.gz'
    with tarfile.open(asset, 'w:gz') as tar:
        info = tarfile.TarInfo('all-smi')
        info.size = len(binary_bytes)
        info.mode = 0o755
        tar.addfile(info, io.BytesIO(binary_bytes))
    return asset


class ReleaseAssetProvenance(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.binary_bytes = b'#!/bin/sh\necho all-smi\n' * 100
        self.binary = root / 'all-smi'
        self.binary.write_bytes(self.binary_bytes)
        self.binary.chmod(0o755)
        self.binary_sha = hashlib.sha256(self.binary_bytes).hexdigest()
        self.asset = make_asset(root, self.binary_bytes)
        self.asset_sha = hashlib.sha256(self.asset.read_bytes()).hexdigest()
        self.evidence = root / 'evidence'
        self.evidence.mkdir()
        self._published = perf_all_smi.PUBLISHED_ASSET_SHA256
        perf_all_smi.PUBLISHED_ASSET_SHA256 = self.asset_sha

    def tearDown(self):
        perf_all_smi.PUBLISHED_ASSET_SHA256 = self._published
        self.tmp.cleanup()

    def test_accepts_binary_from_published_asset(self):
        result = perf_all_smi.verify_release_asset(self.binary, self.binary_sha, self.asset, self.evidence)
        self.assertEqual(result['binary_sha256'], self.binary_sha)
        self.assertEqual(result['asset_sha256'], self.asset_sha)
        self.assertEqual(result['published_asset_sha256'], self.asset_sha)
        self.assertFalse(result['rebuild_equality_required'])
        recorded = json.loads((self.evidence / 'all-smi-provenance.json').read_text())
        self.assertEqual(recorded['provenance'], 'official-fork-release-asset')

    def test_rejects_asset_not_matching_published_checksum(self):
        perf_all_smi.PUBLISHED_ASSET_SHA256 = '0' * 64
        with self.assertRaises(ValueError):
            perf_all_smi.verify_release_asset(self.binary, self.binary_sha, self.asset, self.evidence)

    def test_rejects_binary_not_carried_by_asset(self):
        other = self.binary.parent / 'other'
        other.write_bytes(b'different bytes')
        other.chmod(0o755)
        with self.assertRaises(ValueError):
            perf_all_smi.verify_release_asset(other, hashlib.sha256(b'different bytes').hexdigest(),
                                              self.asset, self.evidence)

    def test_rejects_changed_pinned_binary(self):
        with self.assertRaises(ValueError):
            perf_all_smi.verify_release_asset(self.binary, '0' * 64, self.asset, self.evidence)

    def test_rebuild_equality_is_removed(self):
        self.assertFalse(hasattr(perf_all_smi, 'reproduce'))


if __name__ == '__main__':
    unittest.main()
