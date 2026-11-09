# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""The selected-set boundary rejects omitted, unknown or wrong-lane cases."""
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from execution_evidence import digest
import selected_set


class SelectedSetStrictTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.catalog = self.root / 'catalog.csv'
        self.catalog.write_text('ID,Tier,Parallel group\nBC-GPU-001,RC-GPU,G1\nBC-CPU-001,CPU,C\n')
        self.selection = self.root / 'selection.json'
        self.manifest = self.root / 'manifest.json'
        self.index = self.root / 'index.json'
        self.case = {'candidate_sha': 'a' * 40, 'lane': 'G1', 'cases': {'BC-GPU-001': {}}}

    def setup_files(self, selected, case=None):
        self.selection.write_text(json.dumps({'candidate_sha': 'a' * 40, 'catalog_sha256': digest(self.catalog),
                                              'selected': selected}))
        self.manifest.write_text(json.dumps(case if case is not None else self.case))
        self.index.write_text(json.dumps({'candidate_sha': 'a' * 40, 'selection_sha256': digest(self.selection),
            'catalog_sha256': digest(self.catalog), 'manifests': [{'path': str(self.manifest), 'sha256': digest(self.manifest)}]}))

    def load(self):
        # These structural fixtures deliberately omit production catalog policy
        # columns; policy recomputation is tested separately below.
        with patch('validate_results.load_catalog', return_value={}), \
             patch('validate_results.load_selection'):
            return selected_set.load_set(self.selection, self.catalog, self.index)

    def test_complete_selection_and_wrong_lane(self):
        self.setup_files({'BC-GPU-001': ['core'], 'BC-CPU-001': ['core']})
        with patch.object(selected_set.case_launcher, 'preflight'):
            self.assertEqual(self.load()[0][0], 'BC-GPU-001')
        bad = {**self.case, 'lane': 'G2'}
        self.setup_files({'BC-GPU-001': ['core']}, bad)
        with self.assertRaisesRegex(ValueError, 'catalog lane'):
            self.load()

    def test_unknown_empty_and_non_gpu_only_fail(self):
        for selected, error in (({}, 'empty'), ({'BC-CPU-001': ['core']}, 'GPU case set is empty'),
                                ({'BC-GPU-001': ['core'], 'BC-GPU-999': ['core']}, 'unknown cases')):
            with self.subTest(selected=selected):
                self.setup_files(selected)
                with self.assertRaisesRegex(ValueError, error):
                    self.load()

    def test_selection_is_recomputed_before_any_case_preflight(self):
        import select_cases
        import validate_results
        self.catalog.write_text('ID,Tier,Test case,Selection,Area,Parallel group\n'
            'BC-GPU-001,RC-GPU,fixture one,core,GPU,G1\n'
            'BC-GPU-002,RC-GPU,fixture two,core,GPU,G1\n'
            'BC-CPU-001,RC,fixture three,core,CPU,C\n')
        rows = validate_results.load_catalog(self.catalog)
        selection = select_cases.make_selection(rows, digest(self.catalog), 'a' * 40, 0.1, 3, [], [])
        self.selection.write_text(json.dumps(selection))
        paths = []
        for case_id in ('BC-GPU-001', 'BC-GPU-002'):
            path = self.root / (case_id + '.json')
            path.write_text(json.dumps({'candidate_sha': 'a' * 40, 'lane': 'G1', 'cases': {case_id: {}}}))
            paths.append({'path': str(path), 'sha256': digest(path)})
        def index():
            self.index.write_text(json.dumps({'candidate_sha': 'a' * 40, 'selection_sha256': digest(self.selection),
                'catalog_sha256': digest(self.catalog), 'manifests': paths}))
        index()
        with patch.object(selected_set.case_launcher, 'preflight') as preflight:
            self.assertEqual(len(selected_set.load_set(self.selection, self.catalog, self.index)), 2)
            preflight.reset_mock()
            del selection['selected']['BC-GPU-002']
            self.selection.write_text(json.dumps(selection))
            index()  # An attacker can rehash an edited selection and index.
            with self.assertRaisesRegex(ValueError, 'deterministic policy'):
                selected_set.load_set(self.selection, self.catalog, self.index)
            preflight.assert_not_called()

    def test_duplicate_catalog_and_duplicate_selection_keys_fail(self):
        self.catalog.write_text('ID,Tier,Parallel group\nBC-GPU-001,RC-GPU,G1\nBC-GPU-001,RC-GPU,G1\n')
        self.setup_files({'BC-GPU-001': ['core']})
        with self.assertRaisesRegex(ValueError, 'duplicate IDs'):
            self.load()
        self.catalog.write_text('ID,Tier,Parallel group\nBC-GPU-001,RC-GPU,G1\n')
        self.selection.write_text('{"selected":{},"selected":{}}')
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            self.load()


if __name__ == '__main__':
    unittest.main()
