# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Archive verification logic tested with CPU-only synthetic assets."""
import hashlib
import io
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import perf_baseline


class BaselineTests(unittest.TestCase):
    def test_archive_member_and_binary_hashes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); binary = root / 'binary'; binary.write_bytes(b'fixture')
            archive = root / 'release.tar.gz'
            with tarfile.open(archive, 'w:gz') as tar:
                info = tarfile.TarInfo('metrum-ai-bench-cli-v1.7.2-x86_64-unknown-linux-gnu/bin/metrum-ai-bench-cli-llm')
                info.size = len(b'fixture'); tar.addfile(info, io.BytesIO(b'fixture'))
            digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
            with patch.object(perf_baseline.platform, 'machine', return_value='x86_64'), \
                 patch.dict(perf_baseline.RELEASE, {'x86_64': (digest(archive), digest(binary))}):
                self.assertEqual(perf_baseline.verify(archive, binary)['release'], 'v1.7.2')
                binary.write_bytes(b'different')
                with self.assertRaises(ValueError): perf_baseline.verify(archive, binary)


if __name__ == '__main__': unittest.main()
