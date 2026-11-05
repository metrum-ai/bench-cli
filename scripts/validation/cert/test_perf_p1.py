# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Failure-path wiring for P1; mocked resources are not GPU evidence."""
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import perf_p1_runner as p1


class P1Tests(unittest.TestCase):
    def test_runner_failure_blocks_both_cases(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); evidence = root / 'evidence'; evidence.mkdir()
            (evidence / 'gpu.csv').write_text('NVIDIA RTX PRO 6000 Blackwell Server Edition, fixture-driver\n')
            binary = root / 'binary'; binary.write_bytes(b'fixture')
            telemetry = root / 'telemetry'; telemetry.write_text('fixture')
            manifest = root / 'manifest'; manifest.write_text('{}')
            (evidence / 'all-smi-provenance.json').write_text(json.dumps({
                'source_revision': '3b6eaec3364d3c2eb897bf1433ee93eef609c6a9', 'binary_sha256': 'hash'}))
            config = {'model': 'model', 'revision': 'rev', 'image': 'img',
                      'all_smi_binary': {'sha256': 'hash'}, 'telemetry_config': {'path': str(telemetry), 'sha256': 'hash'},
                      'prompt_sets': {'W1': {'path': str(root / 'W1.jsonl')}}}
            argv = ['test', '--out', str(root), '--candidate', str(binary), '--baseline', str(binary), '--manifest', str(manifest),
                    '--sha', 'a' * 40, '--model', 'model', '--revision', 'rev', '--image', 'img', '--start-utc', '2026-01-01T00:00:00Z',
                    '--owner', 'fixture', '--evidence-link', 'fixture', '--telemetry', str(telemetry)]
            hardware = unittest.mock.Mock(stdout='NVIDIA RTX PRO 6000 Blackwell Server Edition, GPU-fixture, 98304\n')
            with patch('sys.argv', argv), patch.dict(os.environ, {'CUDA_VISIBLE_DEVICES': '0', 'CERT_ATTEMPT': '1'}), \
                 patch.object(p1.manifest_tool, 'load_manifest', return_value=({'candidate_sha': 'a' * 40}, config)), \
                 patch.object(p1.subprocess, 'run', return_value=hardware), patch.object(p1.l1_runner, 'call', return_value=1):
                self.assertEqual(p1.main(), 1)
                records = [json.loads((evidence / f'provisional-{case}.json').read_text()) for case in ('BC-PERF-001', 'BC-TELEM-001')]
                self.assertEqual([record['result'] for record in records], ['FAIL', 'FAIL'])
                self.assertEqual([record['ID'] for record in records], ['BC-PERF-001', 'BC-TELEM-001'])


if __name__ == '__main__': unittest.main()
