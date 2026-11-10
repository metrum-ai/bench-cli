# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic VLM reasoning loopback tests, not case evidence."""
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import freeze_case
import manifest_tool
import rehearse_live_modalities
import rehearse_vlm_reasoning as rehearsal
import r4_model_pins as r4


class GuardTests(unittest.TestCase):
    def test_rejects_wrong_case_before_preflight(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'frozen.json'
            path.write_text('{"cases":{"BC-GPU-999":{}},"lane":"V"}')
            with patch.object(rehearsal.case_launcher, 'preflight') as check:
                with self.assertRaisesRegex(ValueError, 'exactly'):
                    rehearsal.rehearsal(path)
                check.assert_not_called()


@unittest.skipUnless(os.environ.get('CERT_TEST_BIN_DIR'), 'set CERT_TEST_BIN_DIR for product loopback')
class LoopbackTests(unittest.TestCase):
    def test_pinned_image_and_product_with_relative_path_negative(self):
        candidate = Path(os.environ['CERT_TEST_BIN_DIR']).resolve(strict=True) / 'metrum-ai-bench-cli-llm'
        vlm = candidate.parent / 'metrum-ai-bench-cli-vlm'
        image = Path(rehearse_live_modalities.ROOT / 'test-data/vlm/shapes-512.png').resolve(strict=True)
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            prompt = root / 'prompt.jsonl'
            prompt.write_text(''.join(json.dumps({'prompt': f'synthetic prompt {i}', 'image_url': str(image)}) + '\n' for i in range(7)))
            pin = lambda path: {'path': str(path), 'sha256': manifest_tool.sha256_file(path)}
            config = {'model': r4.GENERAL_MODEL, 'prompt_sets': {'default': pin(prompt)},
                      'vlm_image': pin(image), 'vlm_binary': pin(vlm),
                      'server_flags': ['--no-enable-prefix-caching', '--reasoning-parser', 'qwen3']}
            lane = root / 'lane.json'
            lane.write_text(json.dumps({'lanes': {'V': config}}))
            output = root / 'case-output'
            case = rehearsal.CASE
            command = [str(Path(rehearse_live_modalities.__file__).with_name('case_entry.sh')), '--case', case,
                       '--lane', 'v', '--candidate', str(candidate), '--baseline', str(candidate),
                       '--manifest', str(lane), '--out', str(output)]
            pins = [candidate, *freeze_case.harness_sources(Path(rehearsal.__file__).parent)]
            manifest = {'candidate': pin(candidate), 'lane': 'V', 'lane_manifest': pin(lane),
                        'inputs': [pin(path) for path in pins], 'cases': {case: {'command': command, 'out': str(output)}}}
            frozen = root / 'frozen.json'
            frozen.write_text(json.dumps(manifest))
            with patch.object(rehearsal.case_launcher, 'preflight'), \
                 patch.object(rehearsal.manifest_tool, 'load_manifest', return_value=({}, config)):
                rehearsal.rehearsal(frozen)
                prompt.write_text(json.dumps({'prompt': 'synthetic', 'image_url': 'relative.png'}) + '\n')
                config['prompt_sets']['default'] = pin(prompt)
                with self.assertRaisesRegex(ValueError, 'pinned local file'):
                    rehearsal.rehearsal(frozen)
            self.assertFalse(output.exists())


if __name__ == '__main__':
    unittest.main()
