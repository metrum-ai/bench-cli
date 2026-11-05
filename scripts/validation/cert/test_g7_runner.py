# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""G7 frozen-floor and orchestration tests; no GPU or performance evidence."""
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import g7_runner
import manifest_tool


class G7RunnerTests(unittest.TestCase):
    def test_floor_must_precede_run_and_pinned_inputs_rechecked(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); ev = root / 'evidence'; ev.mkdir(); (ev / 'gpu.csv').write_text('fixture, fixture-driver')
            cand = root / 'candidate'; cand.write_text('candidate')
            base = root / 'baseline'; base.write_text('baseline')
            manifest_file = root / 'manifest'; manifest_file.write_text('{}')
            floor = root / 'floor.json'; floor.write_text('{}')
            data = {'candidate_sha': 'a' * 40}
            config = {'model': 'model', 'revision': 'revision', 'image': 'image',
                      'noise_floor': {'path': str(floor), 'sha256': manifest_tool.sha256_file(floor),
                                      'recorded_utc': '2026-01-02T00:00:00Z', 'evidence_link': 'fixture'}}
            argv = ['g7', '--out', str(root), '--candidate', str(cand), '--baseline', str(base), '--manifest', str(manifest_file),
                    '--sha', 'a' * 40, '--model', 'model', '--revision', 'revision', '--image', 'image',
                    '--start-utc', '2026-01-01T00:00:00Z', '--owner', 'fixture', '--evidence-link', 'fixture', '--port', '1', '--proxy-port', '2']
            with patch('sys.argv', argv), patch.dict(os.environ, {'CUDA_VISIBLE_DEVICES': '0'}), \
                 patch.object(g7_runner.manifest_tool, 'load_manifest', return_value=(data, config)) as load, \
                 patch.object(g7_runner, 'run_cell') as run, \
                 patch.object(g7_runner.compare_small, 'compare', return_value={'result': 'PASS'}) as compare, \
                 patch.object(g7_runner.result_writer, 'write_record') as write:
                self.assertEqual(g7_runner.main(), 1)
                self.assertEqual(write.call_args.args[1]['result'], 'FAIL')
                run.assert_not_called(); compare.assert_not_called()
                config['noise_floor']['recorded_utc'] = '2025-12-31T00:00:00Z'
                self.assertEqual(g7_runner.main(), 0)
                self.assertEqual(run.call_count, 12)
                self.assertEqual(load.call_count, 3)
                self.assertEqual(write.call_args.args[1]['result'], 'PASS')
                self.assertIn('evidence/g7-noise-floor.json', write.call_args.args[1]['evidence_paths'])


if __name__ == '__main__': unittest.main()
