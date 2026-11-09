# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic mixed-modality loopback tests, not case evidence."""
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import freeze_case
import manifest_tool
import rehearse_g4_modality as rehearsal
import rehearse_live_modalities


class GuardTests(unittest.TestCase):
    def test_rejects_wrong_case_before_preflight(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'frozen.json'
            path.write_text('{"cases":{"BC-GPU-999":{}},"lane":"G4"}')
            with patch.object(rehearsal.case_launcher, 'preflight') as check:
                with self.assertRaisesRegex(ValueError, 'exactly'):
                    rehearsal.rehearsal(path)
                check.assert_not_called()


@unittest.skipUnless(os.environ.get('CERT_TEST_BIN_DIR'), 'set CERT_TEST_BIN_DIR for product loopback')
class LoopbackTests(unittest.TestCase):
    def test_actual_product_argv_and_relative_image_negative(self):
        candidate = Path(os.environ['CERT_TEST_BIN_DIR']).resolve(strict=True) / 'metrum-ai-bench-cli-llm'
        vlm = candidate.parent / 'metrum-ai-bench-cli-vlm'
        strategic = candidate.parent / 'metrum-ai-bench-cli-strategic'
        image = Path(rehearse_live_modalities.ROOT / 'test-data/vlm/shapes-512.png').resolve(strict=True)
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            prompt, vlm_prompt = root / 'prompt.jsonl', root / 'vlm.jsonl'
            prompt.write_text(''.join(json.dumps({'prompt': f'synthetic prompt {i}'}) + '\n' for i in range(4)))
            vlm_prompt.write_text(''.join(json.dumps({'prompt': f'synthetic prompt {i}', 'image_url': str(image)}) + '\n' for i in range(4)))
            pin = lambda path: {'path': str(path), 'sha256': manifest_tool.sha256_file(path)}
            config = {'model': 'synthetic/model', 'prompt_sets': {'default': pin(prompt), 'vlm': pin(vlm_prompt)},
                      'vlm_image': pin(image), 'vlm_binary': pin(vlm), 'strategic_binary': pin(strategic),
                      'vlm': {'model': 'Qwen/Qwen3-VL-8B-Thinking', 'image': 'vllm/vllm-openai@sha256:' + 'b' * 64,
                              'revision': 'a' * 40, 'server_flags': ['--no-enable-prefix-caching', '--reasoning-parser', 'qwen3']}}
            lane = root / 'lane.json'
            lane.write_text(json.dumps({'lanes': {'G4': config}}))
            output = root / 'case-output'
            case = rehearsal.CASE
            command = [str(Path(rehearse_live_modalities.__file__).with_name('case_entry.sh')), '--case', case,
                       '--lane', 'g4', '--candidate', str(candidate), '--baseline', str(candidate),
                       '--manifest', str(lane), '--out', str(output)]
            pins = [candidate, *freeze_case.harness_sources(Path(rehearsal.__file__).parent)]
            manifest = {'candidate': pin(candidate), 'lane': 'G4', 'lane_manifest': pin(lane),
                        'inputs': [pin(path) for path in pins], 'cases': {case: {'command': command, 'out': str(output)}}}
            frozen = root / 'frozen.json'
            frozen.write_text(json.dumps(manifest))
            default_report = rehearse_live_modalities.ROOT / 'metrum-ai-bench-cli-report.html'
            before = default_report.read_bytes() if default_report.exists() else None
            with patch.object(rehearsal.case_launcher, 'preflight'), \
                 patch.object(rehearsal.manifest_tool, 'load_manifest', return_value=({}, config)):
                rehearsal.rehearsal(frozen)
                self.assertEqual(default_report.read_bytes() if default_report.exists() else None, before)
                vlm_prompt.write_text(json.dumps({'prompt': 'synthetic', 'image_url': 'relative.png'}) + '\n')
                config['prompt_sets']['vlm'] = pin(vlm_prompt)
                with self.assertRaisesRegex(ValueError, 'pinned local file'):
                    rehearsal.rehearsal(frozen)
            self.assertFalse(output.exists())


if __name__ == '__main__':
    unittest.main()
