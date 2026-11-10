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
import result_writer


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
            first = json.loads(order_path.read_text().splitlines()[0])
            self.assertEqual(first['prompt_bytes'], len(self.rows[0]['prompt'].encode()))
            self.assertEqual(first['cohort'], 0)
            self.assertEqual(json.loads(order_path.read_text().splitlines()[-1])['cohort'], 63)
            receipts.append(receipt)
        for key in ('reference', 'reference_sharegpt'):
            self.assertEqual(receipts[0]['comparison_config']['prompt_sets'][key]['sha256'],
                             receipts[1]['comparison_config']['prompt_sets'][key]['sha256'])
        self.assertEqual(receipts[0]['comparison_config']['reference_order']['sha256'],
                         receipts[1]['comparison_config']['reference_order']['sha256'])
        with self.assertRaises(FileExistsError):
            prepare_reference_inputs.prepare(self.source, sha, self.root / 'one')

    def test_rejects_missing_or_incorrect_cohort_before_freeze(self):
        receipt_pin = prepare_reference_inputs.prepare(self.source, manifest_tool.sha256_file(self.source), self.root / 'valid')
        config = json.loads(Path(receipt_pin['path']).read_text())['comparison_config']
        path = Path(config['reference_order']['path'])
        original = [json.loads(line) for line in path.read_text().splitlines()]
        for value in (None, True, 99):
            rows = [dict(row) for row in original]
            if value is None:
                del rows[1]['cohort']
            else:
                rows[1]['cohort'] = value
            path.write_text(''.join(json.dumps(row) + '\n' for row in rows))
            config['reference_order']['sha256'] = manifest_tool.sha256_file(path)
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, 'cohorts'):
                comp_reference_runner.reference_inputs(config)

    def test_marked_source_never_enters_certification_inputs(self):
        smoke = self.root / 'smoke'; smoke.mkdir()
        (smoke / result_writer.SMOKE_MARKER).write_text(json.dumps(
            {'classification': result_writer.SMOKE_CLASS, 'expectations_sha256': 'f' * 64}))
        source = smoke / 'source.jsonl'; source.write_bytes(self.source.read_bytes())
        out = self.root / 'cert-input'
        with self.assertRaises(result_writer.NonCertificationEvidence):
            prepare_reference_inputs.prepare(source, manifest_tool.sha256_file(source), out)
        self.assertFalse(out.exists())
        # Synthetic same-root preparation is still possible, as is using a
        # frozen ordinary input for the isolated smoke preparation.
        prepare_reference_inputs.prepare(source, manifest_tool.sha256_file(source), smoke / 'same')
        prepare_reference_inputs.prepare(self.source, manifest_tool.sha256_file(self.source), smoke / 'frozen')

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
