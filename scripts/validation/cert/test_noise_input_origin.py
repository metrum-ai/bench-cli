# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic noise-floor origin controls; no GPU or certification execution."""
import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import g7_runner
import perf_p1_runner
import perf_p2_runner
import result_writer


class NoiseOriginTests(unittest.TestCase):
    def test_each_runner_rejects_smoke_noise_before_copy_or_gpu(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); smoke = root / 'smoke'; smoke.mkdir()
            (smoke / result_writer.SMOKE_MARKER).write_text(json.dumps(
                {'classification': result_writer.SMOKE_CLASS, 'expectations_sha256': 'f' * 64}))
            floor = smoke / 'floor.json'; floor.write_text('{}')
            binary = root / 'binary'; binary.write_text('fixture')
            manifest_file = root / 'manifest'; manifest_file.write_text('{}')
            telemetry = root / 'telemetry'; telemetry.write_text('{}')
            config = {'model': 'model', 'revision': 'rev', 'image': 'image',
                      'telemetry_config': {'path': str(telemetry), 'sha256': 'd' * 64},
                      'all_smi_binary': {'sha256': 'b' * 64},
                      'noise_floor': {'path': str(floor), 'sha256': 'c' * 64,
                                      'recorded_utc': '2026-01-01T00:00:00Z', 'evidence_link': 'fixture'}}
            manifest = {'candidate_sha': 'a' * 40}
            for lane, module in [('P1', perf_p1_runner), ('P2', perf_p2_runner), ('G7', g7_runner)]:
                with self.subTest(lane=lane):
                    out = root / lane; ev = out / 'evidence'; ev.mkdir(parents=True)
                    (ev / 'gpu.csv').write_text('fixture-gpu, fixture-driver')
                    (ev / 'all-smi-provenance.json').write_text(json.dumps(
                        {'provenance': 'official-fork-release-asset', 'binary_sha256': 'b' * 64,
                         'release_tag': 'fixture', 'asset_name': 'fixture', 'asset_sha256': 'c' * 64}))
                    argv = ['runner', '--out', str(out), '--candidate', str(binary), '--baseline', str(binary),
                            '--manifest', str(manifest_file), '--sha', 'a' * 40, '--model', 'model',
                            '--revision', 'rev', '--image', 'image', '--start-utc', '2026-01-02T00:00:00Z',
                            '--owner', 'fixture', '--evidence-link', 'fixture']
                    if lane == 'P1': argv += ['--telemetry', str(telemetry)]
                    if lane == 'G7': argv += ['--port', '10000', '--proxy-port', '10001']
                    with patch('sys.argv', argv), patch.dict('os.environ', {'CUDA_VISIBLE_DEVICES': '0'}), \
                         patch.object(module.manifest_tool, 'load_manifest', return_value=(manifest, config)), \
                         patch.object(module.subprocess, 'run') as run, contextlib.redirect_stderr(io.StringIO()):
                        self.assertEqual(module.main(), 1)
                        run.assert_not_called()
                    self.assertFalse((ev / f'{lane.lower()}-noise-floor.json').exists())
                    self.assertFalse(any(ev.glob('provisional-*.json')))
                    self.assertFalse((out / 'results').exists())


if __name__ == '__main__':
    unittest.main()
