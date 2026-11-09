# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Case-specific frozen-input checks; not a full launch rehearsal."""
import hashlib
import json
import tempfile
import unittest
from pathlib import Path

import case_input_checks
import manifest_tool


class CaseInputChecksTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.candidate = self.root / 'metrum-ai-bench-cli-llm'
        self.candidate.write_text('candidate')
        self.strategic = self.root / 'metrum-ai-bench-cli-strategic'
        self.strategic.write_text('strategic')
        self.strategic.chmod(0o755)
        self.image = self.root / 'image.png'
        self.image.write_bytes(b'image')
        self.prompt = self.root / 'prompts.jsonl'
        self.prompt.write_text(json.dumps({'prompt': 'test', 'image_url': str(self.image)}) + '\n')

    def pin(self, path):
        return {'path': str(path), 'sha256': manifest_tool.sha256_file(path)}

    def test_g1_requires_tool_pin_and_flags(self):
        config = {'server_flags': ['--enable-auto-tool-choice', '--tool-call-parser', 'hermes'],
                  'prompt_sets': {'tool': self.pin(self.prompt)}}
        case_input_checks.check('BC-GPU-003', config, self.candidate)
        self.prompt.write_text('{"prompt":""}\n')
        config['prompt_sets']['tool'] = self.pin(self.prompt)
        with self.assertRaises(ValueError):
            case_input_checks.check('BC-GPU-003', config, self.candidate)
        del config['prompt_sets']['tool']
        with self.assertRaises(KeyError):
            case_input_checks.check('BC-GPU-003', config, self.candidate)

    def test_vlm_image_and_g5_inputs(self):
        config = {'vlm_image': self.pin(self.image), 'prompt_sets': {'default': self.pin(self.prompt)}}
        case_input_checks.check('BC-REASON-005', config, self.candidate)
        self.prompt.write_text('{"prompt":"test","image_url":"image.png"}\n')
        config['prompt_sets']['default'] = self.pin(self.prompt)
        with self.assertRaises(ValueError):
            case_input_checks.check('BC-REASON-005', config, self.candidate)
        config = {'strategic_binary': self.pin(self.strategic), 'port': 18000, 'proxy_port': 18100,
                  'second_port': 18001, 'second_proxy_port': 18101,
                  'server_flags': ['--gpu-memory-utilization', '0.45']}
        case_input_checks.check('BC-GPU-013', config, self.candidate)
        case_input_checks.check('BC-GPU-015', config, self.candidate)
        config['second_proxy_port'] = 18000
        with self.assertRaises(ValueError):
            case_input_checks.check('BC-GPU-015', config, self.candidate)
        config['strategic_binary']['sha256'] = '0' * 64
        with self.assertRaises(ValueError):
            case_input_checks.check('BC-GPU-013', config, self.candidate)

    def test_reference_sequence_reuses_runner_validation(self):
        prompts, sharegpt, order = [], [], []
        for index in range(64):
            text = f'prompt {index}'
            prompts.append({'prompt': text})
            sharegpt.append({'conversations': [{'value': text}, {'value': 'placeholder'}]})
            order.append({'seq': index, 'prompt_index': index,
                          'prompt_sha256_16': hashlib.sha256(text.encode()).hexdigest()[:16],
                          'prompt_bytes': len(text.encode()), 'cohort': index})
        prompt, dataset, slots = self.root / 'reference.jsonl', self.root / 'sharegpt.json', self.root / 'order.jsonl'
        prompt.write_text(''.join(json.dumps(row) + '\n' for row in prompts))
        dataset.write_text(json.dumps(sharegpt))
        slots.write_text(''.join(json.dumps(row) + '\n' for row in order))
        config = {'comparison_cap': 1024, 'prompt_sets': {'reference': self.pin(prompt),
                  'reference_sharegpt': self.pin(dataset)}, 'reference_order': self.pin(slots)}
        case_input_checks.check('BC-COMP-002', config, self.candidate)
        sharegpt[0]['conversations'][0]['value'] = 'different'
        dataset.write_text(json.dumps(sharegpt))
        config['prompt_sets']['reference_sharegpt'] = self.pin(dataset)
        with self.assertRaises(ValueError):
            case_input_checks.check('BC-COMP-002', config, self.candidate)


if __name__ == '__main__':
    unittest.main()
