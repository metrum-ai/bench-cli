# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Reference preparation tests; synthetic inputs are never certification evidence."""
import json
import tempfile
import unittest
from pathlib import Path

import comp_reference_runner
import manifest_tool
import prepare_reference_inputs


class ReferencePreparationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / 'source.jsonl'
        self.rows = [{'prompt': f'Synthetic prompt {i} café'} for i in range(64)]
        self.write()

    def write(self):
        self.source.write_text(''.join(json.dumps(row, ensure_ascii=False) + '\n' for row in self.rows), encoding='utf-8')

    def test_runner_accepts_reproducible_pinned_outputs(self):
        sha = manifest_tool.sha256_file(self.source)
        receipts = []
        for name in ('one', 'two'):
            pin = prepare_reference_inputs.prepare(self.source, sha, self.root / name)
            self.assertEqual(pin['sha256'], manifest_tool.sha256_file(pin['path']))
            receipt = json.loads(Path(pin['path']).read_text())
            prompts, sharegpt, order_path, order = comp_reference_runner.reference_inputs(receipt['comparison_config'])
            self.assertEqual(len(order), 64)
            self.assertEqual(json.loads(prompts.read_text().splitlines()[0])['prompt'], self.rows[0]['prompt'])
            self.assertEqual(json.loads(sharegpt.read_text())[0]['conversations'][0]['value'], self.rows[0]['prompt'])
            self.assertEqual(json.loads(order_path.read_text().splitlines()[0])['prompt_bytes'], len(self.rows[0]['prompt'].encode()))
            receipts.append(receipt)
        for key in ('reference', 'reference_sharegpt'):
            self.assertEqual(receipts[0]['comparison_config']['prompt_sets'][key]['sha256'],
                             receipts[1]['comparison_config']['prompt_sets'][key]['sha256'])
        self.assertEqual(receipts[0]['comparison_config']['reference_order']['sha256'],
                         receipts[1]['comparison_config']['reference_order']['sha256'])
        with self.assertRaises(FileExistsError):
            prepare_reference_inputs.prepare(self.source, sha, self.root / 'one')

    def test_invalid_source_fails_before_creating_output(self):
        output = self.root / 'output'
        with self.assertRaisesRegex(ValueError, 'hash mismatch'):
            prepare_reference_inputs.prepare(self.source, '0' * 64, output)
        self.assertFalse(output.exists())
        for rows in (self.rows[:63], self.rows[:63] + [self.rows[0]], self.rows[:63] + [{'prompt': ''}]):
            self.rows = rows
            self.write()
            with self.subTest(rows=len(rows)), self.assertRaises(ValueError):
                prepare_reference_inputs.prepare(self.source, manifest_tool.sha256_file(self.source), output)
            self.assertFalse(output.exists())


if __name__ == '__main__':
    unittest.main()
