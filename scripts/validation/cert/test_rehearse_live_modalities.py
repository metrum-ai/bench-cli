# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""CPU-only rehearsal tests; synthetic fixtures never become case evidence."""
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import freeze_case
import manifest_tool
import prepare_product_sut
import rehearse_live_modalities as rehearsal


class FrozenArgvTests(unittest.TestCase):
    def test_frozen_argv_and_harness_pins_fail_closed(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            candidate = root / 'candidate'
            candidate.write_text('synthetic')
            lane = root / 'lane.json'
            lane.write_text('{}')
            output = root / 'result'
            case = 'BC-LIVE-002'
            command = [str(Path(rehearsal.__file__).with_name('case_entry.sh')), '--case', case,
                       '--lane', 'l2', '--candidate', str(candidate), '--baseline', str(candidate),
                       '--manifest', str(lane), '--out', str(output)]
            pins = [candidate, *freeze_case.harness_sources(Path(rehearsal.__file__).parent)]
            manifest = {'candidate': {'path': str(candidate)}, 'lane_manifest': {'path': str(lane)},
                        'inputs': [{'path': str(p), 'sha256': manifest_tool.sha256_file(p)} for p in pins],
                        'cases': {case: {'command': command, 'out': str(output)}}}
            self.assertEqual(rehearsal.parse_frozen_command(manifest, case), candidate.resolve())
            command[-1] = str(root / 'unexpected')
            with self.assertRaisesRegex(ValueError, 'output'):
                rehearsal.parse_frozen_command(manifest, case)
            command[-1] = str(output)
            manifest['inputs'][1]['sha256'] = '0' * 64
            with self.assertRaisesRegex(ValueError, 'harness source'):
                rehearsal.parse_frozen_command(manifest, case)

    def test_rejects_unknown_case_before_external_actions(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'manifest.json'
            path.write_text('{"cases":{"BC-GPU-999":{}}}')
            with patch.object(rehearsal.case_launcher, 'preflight') as preflight:
                with self.assertRaisesRegex(ValueError, 'supported'):
                    rehearsal.rehearsal(path, 'BC-GPU-999')
                preflight.assert_not_called()


@unittest.skipUnless(os.environ.get('CERT_TEST_BIN_DIR'), 'set CERT_TEST_BIN_DIR for real-binary loopback checks')
class LiveLoopbackTests(unittest.TestCase):
    def test_all_three_actual_xtask_and_product_commands(self):
        # Synthetic manifests intentionally patch only source/build-receipt
        # verification; CLI/xtask execution and product parsers are real.
        binary_dir = Path(os.environ['CERT_TEST_BIN_DIR']).resolve(strict=True)
        candidate = binary_dir / 'metrum-ai-bench-cli-llm'
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            actual = {'gpus': [{'name': 'Synthetic GPU', 'memory_gb': 48, 'driver': 'synthetic'}],
                      'memory_gb': 64, 'host_os': 'Synthetic Linux'}
            for case, (lane, modality) in rehearsal.MODALITIES.items():
                with self.subTest(case=case):
                    config = {'model': 'synthetic/model', 'revision': 'a' * 40,
                              'image': 'vllm/vllm-openai@sha256:' + 'b' * 64, 'server_flags': []}
                    sut = root / f'{lane}-sut.json'
                    sut.write_text(json.dumps(prepare_product_sut.declaration(actual, config)))
                    prompt = root / f'{lane}-prompt.txt'
                    prompt.write_text('synthetic image prompt')
                    config.update({'sut': {'path': str(sut), 'sha256': manifest_tool.sha256_file(sut)},
                                   'prompt_sets': {'default': {'path': str(prompt), 'sha256': manifest_tool.sha256_file(prompt)}}})
                    lane_path = root / f'{lane}.json'
                    lane_path.write_text(json.dumps({'lanes': {lane: config}}))
                    pins = [candidate, *freeze_case.harness_sources(Path(rehearsal.__file__).parent)]
                    out = root / f'{lane}-out'
                    command = [str(Path(rehearsal.__file__).with_name('case_entry.sh')), '--case', case,
                               '--lane', lane.lower(), '--candidate', str(candidate), '--baseline', str(candidate),
                               '--manifest', str(lane_path), '--out', str(out)]
                    manifest = {'candidate': {'path': str(candidate), 'sha256': manifest_tool.sha256_file(candidate)},
                                'lane_manifest': {'path': str(lane_path), 'sha256': manifest_tool.sha256_file(lane_path)},
                                'inputs': [{'path': str(p), 'sha256': manifest_tool.sha256_file(p)} for p in pins],
                                'cases': {case: {'command': command, 'out': str(out)}}}
                    path = root / f'{case}.json'
                    path.write_text(json.dumps(manifest))
                    with patch.object(rehearsal.case_launcher, 'preflight'), \
                         patch.object(rehearsal.manifest_tool, 'load_manifest', return_value=({}, config)):
                        rehearsal.rehearsal(path, case)
                        if case == 'BC-LIVE-002':
                            malformed = json.loads(sut.read_text())
                            malformed['gpu']['name'] = malformed['gpu'].pop('model')
                            sut.write_text(json.dumps(malformed))
                            with self.assertRaisesRegex(ValueError, 'live vlm command failed'):
                                rehearsal.rehearsal(path, case)
                    self.assertFalse(out.exists(), 'rehearsal must not write the frozen case output')


if __name__ == '__main__':
    unittest.main()
