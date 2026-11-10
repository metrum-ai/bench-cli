# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Prospective r4 model pins; no live GPU or certification claim."""
import csv
import hashlib
import unittest
from pathlib import Path

CATALOG = Path(__file__).resolve().parents[3] / 'docs' / 'RELEASE_CERTIFICATION_CATALOG.csv'
GENERAL = 'Qwen/Qwen3.8-27B-FP8 revision c3a462cc3d38cf351f3b26cbd763469eeb74ef81'
SMALL = 'Qwen/Qwen3.5-0.8B revision 2fc06364715b967f1860aea9cf38778875588b17'
IMAGE = 'vllm/vllm-openai@sha256:d825baf9ce4b1b6bfc5ecc7b79880c7e6748c65a72d697ce3d47607aafa84c6e'


class CatalogR4Tests(unittest.TestCase):
    def test_models_and_previous_release_gate(self):
        text = CATALOG.read_text(encoding='utf-8')
        rows = list(csv.DictReader(text.splitlines()))
        cases = {row['ID']: row for row in rows}
        self.assertEqual(len(cases), len(rows))
        raw_perf = next(line for line in text.splitlines() if line.startswith('BC-PERF-001,'))
        self.assertEqual(hashlib.sha256(raw_perf.encode()).hexdigest(),
                         'c12ba7c50f0d2c3156be8079020f0e4cb2b7326d07009fa7f309b495d9d4ad8a')
        small = {'BC-GPU-015', 'BC-PERF-003'}
        general = {'BC-LIVE-001', 'BC-LIVE-002', 'BC-REASON-001', 'BC-REASON-002', 'BC-REASON-003',
                   'BC-REASON-004', 'BC-REASON-005', 'BC-REASON-006', 'BC-HW-001', 'BC-COMP-002',
                   'BC-COMP-003', 'BC-INTEG-001'} | {f'BC-GPU-{n:03}' for n in range(1, 17)} - {'BC-GPU-015'}
        for case in small | general:
            with self.subTest(case=case):
                self.assertIn(SMALL if case in small else GENERAL, cases[case]['Model/server'])
                self.assertIn(IMAGE, cases[case]['Model/server'])
        for row in rows:
            self.assertNotIn('Qwen/Qwen3-8B', str(row))
            self.assertNotIn('Qwen/Qwen3-4B', str(row))
        self.assertIn('image', cases['BC-GPU-011']['Model/server'])
        self.assertIn('image_count>0', cases['BC-LIVE-002']['Pass criteria'])
        self.assertIn('0.45', cases['BC-GPU-015']['Command or procedure'])
        self.assertIn('sub-10B', cases['BC-PERF-003']['What it proves'])


if __name__ == '__main__':
    unittest.main()
