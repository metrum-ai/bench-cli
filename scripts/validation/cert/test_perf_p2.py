# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""CPU-only wiring checks; fabricated fixtures are never certification evidence."""
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import perf_p2_runner as p2


class PerfP2Tests(unittest.TestCase):
    def test_runner_failure_cannot_pass(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); evidence = root / 'evidence'; evidence.mkdir()
            (evidence / 'gpu.csv').write_text('NVIDIA RTX PRO 6000 Blackwell Server Edition, fixture-driver\n')
            binary = root / 'binary'; binary.write_bytes(b'fixture')
            manifest = root / 'manifest'; manifest.write_text('{}')
            config = {'model': 'model', 'revision': 'rev', 'image': 'img', 'prompt_sets': {'W1': {'path': str(root / 'W1.jsonl')}}}
            argv = ['test', '--out', str(root), '--candidate', str(binary), '--baseline', str(binary), '--manifest', str(manifest),
                    '--sha', 'a' * 40, '--model', 'model', '--revision', 'rev', '--image', 'img', '--start-utc', '2026-01-01T00:00:00Z',
                    '--owner', 'fixture', '--evidence-link', 'fixture']
            hardware = unittest.mock.Mock(stdout='NVIDIA RTX PRO 6000 Blackwell Server Edition, GPU-fixture, 98304\n')
            with patch('sys.argv', argv), patch.dict(os.environ, {'CUDA_VISIBLE_DEVICES': '0', 'CERT_ATTEMPT': '1'}), \
                 patch.object(p2.manifest_tool, 'load_manifest', return_value=({'candidate_sha': 'a' * 40}, config)), \
                 patch.object(p2.subprocess, 'run', return_value=hardware), patch.object(p2.l1_runner, 'call', return_value=1), \
                 patch.object(p2.result_writer, 'write_record') as result:
                self.assertEqual(p2.main(), 1)
                self.assertEqual(result.call_args.args[1]['result'], 'FAIL')
                self.assertIn('cleanup', result.call_args.args[1]['observed_values']['error'])
                config['noise_floor'] = {'path': str(manifest), 'sha256': '0' * 64,
                                         'recorded_utc': '2026-01-02T00:00:00Z', 'evidence_link': 'fixture'}
                self.assertEqual(p2.main(), 1)
                self.assertIn('prior timestamp', result.call_args.args[1]['observed_values']['error'])


if __name__ == '__main__': unittest.main()
