# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic comparison candidate loopback; reference CLI is not exercised."""
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import freeze_case
import manifest_tool
import prepare_reference_inputs
import rehearse_comp_candidate as rehearsal
import rehearse_live_modalities


class GuardTests(unittest.TestCase):
    def test_unknown_case_fails_before_preflight(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'frozen.json'
            path.write_text('{"cases":{"BC-GPU-999":{}},"lane":"D"}')
            with patch.object(rehearsal.case_launcher, 'preflight') as check:
                with self.assertRaisesRegex(ValueError, 'exactly'):
                    rehearsal.rehearsal(path, temp)
                check.assert_not_called()


@unittest.skipUnless(os.environ.get('CERT_TEST_LLM'), 'set CERT_TEST_LLM for product loopback')
class LoopbackTests(unittest.TestCase):
    def test_pinned_candidate_order_and_bad_tokenizer(self):
        candidate = Path(os.environ['CERT_TEST_LLM']).resolve(strict=True)
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / 'source.jsonl'
            source.write_text(''.join(json.dumps({'prompt': f'synthetic comparison prompt {i}'}) + '\n' for i in range(64)))
            pin = lambda path: {'path': str(path), 'sha256': manifest_tool.sha256_file(path)}
            receipt = prepare_reference_inputs.prepare(source, manifest_tool.sha256_file(source), root / 'inputs')
            config = json.loads(Path(receipt['path']).read_text())['comparison_config']
            config.update({'model': 'synthetic/model', 'revision': 'a' * 40,
                           'image': 'vllm/vllm-openai@sha256:' + 'b' * 64})
            config['prompt_sets']['default'] = config['prompt_sets']['reference']
            cache = root / 'cache'
            snapshot = cache / 'models--synthetic--model' / 'snapshots' / ('a' * 40)
            snapshot.mkdir(parents=True)
            tokenizer = snapshot / 'tokenizer.json'
            tokenizer.write_text('{}')
            lane = root / 'lane.json'
            lane.write_text(json.dumps({'lanes': {'D': config}}))
            output = root / 'case-output'
            case = rehearsal.CASE
            command = [str(Path(rehearse_live_modalities.__file__).with_name('case_entry.sh')), '--case', case,
                       '--lane', 'd', '--candidate', str(candidate), '--baseline', str(candidate),
                       '--manifest', str(lane), '--out', str(output)]
            pins = [candidate, *freeze_case.harness_sources(Path(rehearsal.__file__).parent)]
            manifest = {'candidate': pin(candidate), 'tokenizer': {**pin(tokenizer), 'revision': 'a' * 40},
                        'lane': 'D', 'lane_manifest': pin(lane), 'inputs': [pin(path) for path in pins],
                        'cases': {case: {'command': command, 'out': str(output)}}}
            frozen = root / 'frozen.json'
            frozen.write_text(json.dumps(manifest))
            with patch.object(rehearsal.case_launcher, 'preflight'), \
                 patch.object(rehearsal.manifest_tool, 'load_manifest', return_value=({}, config)):
                rehearsal.rehearsal(frozen, cache)
                tokenizer.write_text('{"changed":true}')
                with self.assertRaisesRegex(ValueError, 'snapshot does not match'):
                    rehearsal.rehearsal(frozen, cache)
            self.assertFalse(output.exists())


if __name__ == '__main__':
    unittest.main()
