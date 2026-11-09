#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Selection/box binding with mocked production-set preflight; no GPU work."""
import hashlib
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import stage0_plan


def digest(path): return hashlib.sha256(path.read_bytes()).hexdigest()


class PlanTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name); self.candidate = 'a' * 40
        self.selection = self.root / 'selection.json'; self.selection.write_text(json.dumps({'candidate_sha': self.candidate}))
        self.catalog = self.root / 'catalog.csv'; self.catalog.write_text('fixture')
        self.index = self.root / 'index.json'; self.index.write_text(json.dumps({'candidate_sha': self.candidate, 'deferred_at_trigger': []}))
        self.case = 'BC-GPU-001'
        self.manifest = self.root / 'manifest.json'
        self.manifest.write_text(json.dumps({'candidate_sha': self.candidate, 'cases': {self.case: {}}}))
        self.item = {'id': self.case, 'allowed_classes': ['COMPLETE'], 'cap_seconds': 150, 'grace_seconds': 30,
                     'manifest': {'path': str(self.manifest), 'sha256': digest(self.manifest)}, 'request_log': 'evidence/request.jsonl'}
        self.plan = {'candidate_sha': self.candidate, 'execution_index_sha256': digest(self.index),
                     'selection_sha256': digest(self.selection), 'catalog_sha256': digest(self.catalog),
                     'boxes': [{'id': 'box-a', 'gpu': 'expected GPU', 'planned_seconds': 160,
                                'hard_cap_seconds': 200, 'cases': [self.item]}]}
        self.expectations = self.root / 'expectations.json'
        self.entries = [(self.case, self.manifest, {}, digest(self.manifest))]

    def run_validate(self):
        self.expectations.write_text(json.dumps(self.plan))
        with patch.object(stage0_plan.selected_set, 'load_set', return_value=self.entries) as load:
            result = stage0_plan.validate(self.expectations, digest(self.expectations), selection_path=self.selection,
                catalog_path=self.catalog, index_path=self.index, approved_index_sha=digest(self.index))
            self.assertEqual(load.call_args.kwargs['allow_pretrigger'], False)
            return result

    def test_exact_selected_set_and_manifest(self):
        self.assertEqual(self.run_validate(), {self.case: 'box-a'})

    def test_omission_duplicate_and_unselected_case_fail(self):
        self.entries.append(('BC-GPU-002', self.manifest, {}, digest(self.manifest)))
        with self.assertRaisesRegex(ValueError, 'assignments differ'): self.run_validate()
        self.entries.pop()
        box = self.plan['boxes'][0].copy(); box['id'] = 'box-b'
        self.plan['boxes'].append(box)
        with self.assertRaisesRegex(ValueError, 'more than one box'): self.run_validate()

    def test_hard_cap_and_case_budget_fail_closed(self):
        self.plan['boxes'][0]['hard_cap_seconds'] = 201
        with self.assertRaisesRegex(ValueError, '1.25'): self.run_validate()
        self.plan['boxes'][0]['hard_cap_seconds'] = 200
        self.item['cap_seconds'] = 161
        with self.assertRaisesRegex(ValueError, 'exceed planned'): self.run_validate()

    def test_index_and_manifest_hash_mismatch_fail(self):
        self.plan['execution_index_sha256'] = 'b' * 64
        with self.assertRaisesRegex(ValueError, 'approved digest'): self.run_validate()
        self.plan['execution_index_sha256'] = digest(self.index)
        self.item['manifest']['sha256'] = 'b' * 64
        with self.assertRaisesRegex(ValueError, 'approved execution index'): self.run_validate()


if __name__ == '__main__': unittest.main()
