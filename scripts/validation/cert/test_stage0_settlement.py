#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Fake create receipt matching; no live daemon settlement proof."""
import json
import tempfile
import unittest
from pathlib import Path

import r4_image_provenance as r4
import stage0_settlement


class SettlementTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name); self.evidence = self.root / 'evidence'; self.evidence.mkdir()
        self.uuid = 'GPU-11111111-1111-1111-1111-111111111111'
        (self.evidence / 'gpu.csv').write_text('NVIDIA H100,550.1\n')
        (self.evidence / 'container-id.txt').write_text('a' * 64 + '\n')
        owner = self.evidence / 'server-container-owner.txt'; owner.write_text('b' * 32 + '\n'); owner.chmod(0o600)
        self.receipt = {'image_index': r4.INDEX, 'platform_manifest': r4.AMD64, 'image_config': r4.CONFIG,
                        'platform': 'linux/amd64', 'container_verified': True, 'gpu_uuid': self.uuid,
                        'container_id': 'a' * 64, 'container_owner': 'b' * 32,
                        'gpu_visible': {'name': 'NVIDIA H100', 'uuid': self.uuid, 'driver': '550.1'}}
        self.path = self.evidence / 'r4-image-runtime.json'; self.path.write_text(json.dumps(self.receipt))

    def check(self):
        return stage0_settlement.successful_create(self.root, gpu_name='NVIDIA H100', gpu_uuid=self.uuid)

    def test_matching_create_and_inspect(self):
        self.assertTrue(self.check())
        self.receipt['container_id'] = 'c' * 64; self.path.write_text(json.dumps(self.receipt))
        with self.assertRaisesRegex(ValueError, 'runtime container identity'): self.check()

    def test_missing_or_noncanonical_response_is_not_settled(self):
        for data in ('', 'a' * 12 + '\n', 'a' * 64, 'a' * 64 + '\nextra'):
            (self.evidence / 'container-id.txt').write_text(data)
            with self.subTest(data=data), self.assertRaises(ValueError): self.check()
        (self.evidence / 'container-id.txt').unlink()
        with self.assertRaises(ValueError): self.check()


if __name__ == '__main__': unittest.main()
