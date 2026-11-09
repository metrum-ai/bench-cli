# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic multi-endpoint loopback tests, not case evidence."""
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import freeze_case
import manifest_tool
import rehearse_g5_endpoints as rehearsal
import rehearse_live_modalities


class GuardTests(unittest.TestCase):
    def test_unknown_case_fails_before_preflight(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'frozen.json'
            path.write_text('{"cases":{"BC-GPU-999":{}},"lane":"G5"}')
            with patch.object(rehearsal.case_launcher, 'preflight') as check:
                with self.assertRaisesRegex(ValueError, 'exactly'):
                    rehearsal.rehearsal(path)
                check.assert_not_called()


@unittest.skipUnless(os.environ.get('CERT_TEST_LLM'), 'set CERT_TEST_LLM for real product loopback')
class LoopbackTests(unittest.TestCase):
    def test_product_and_proxy_endpoints_with_four_port_negative(self):
        candidate = Path(os.environ['CERT_TEST_LLM']).resolve(strict=True)
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            prompt = root / 'prompt.jsonl'
            prompt.write_text(''.join(json.dumps({'prompt': f'synthetic prompt {i}'}) + '\n' for i in range(64)))
            pin = lambda path: {'path': str(path), 'sha256': manifest_tool.sha256_file(path)}
            config = {'model': 'synthetic/model', 'prompt_sets': {'default': pin(prompt)},
                      'port': 8000, 'proxy_port': 8001, 'second_port': 8002, 'second_proxy_port': 8003,
                      'server_flags': ['--gpu-memory-utilization', '0.45']}
            lane = root / 'lane.json'
            lane.write_text(json.dumps({'lanes': {'G5': config}}))
            output = root / 'case-output'
            case = rehearsal.CASE
            command = [str(Path(rehearse_live_modalities.__file__).with_name('case_entry.sh')), '--case', case,
                       '--lane', 'g5', '--candidate', str(candidate), '--baseline', str(candidate),
                       '--manifest', str(lane), '--out', str(output)]
            pins = [candidate, *freeze_case.harness_sources(Path(rehearsal.__file__).parent)]
            manifest = {'candidate': pin(candidate), 'lane': 'G5', 'lane_manifest': pin(lane),
                        'inputs': [pin(path) for path in pins], 'cases': {case: {'command': command, 'out': str(output)}}}
            frozen = root / 'frozen.json'
            frozen.write_text(json.dumps(manifest))
            with patch.object(rehearsal.case_launcher, 'preflight'), \
                 patch.object(rehearsal.manifest_tool, 'load_manifest', return_value=({}, config)):
                rehearsal.rehearsal(frozen)
                config['second_proxy_port'] = config['port']
                with self.assertRaisesRegex(ValueError, 'four distinct'):
                    rehearsal.rehearsal(frozen)
            self.assertFalse(output.exists())


if __name__ == '__main__':
    unittest.main()
