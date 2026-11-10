# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""The selected-set boundary rejects omitted, unknown or wrong-lane cases."""
import datetime as dt
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
        self.catalog.write_text('ID,Tier,Parallel group\nBC-GPU-001,RC-GPU,G1\nBC-GPU-002,RC-GPU,G2\nBC-CPU-001,CPU,C\n')
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

    def test_approved_na_is_validated_and_never_launched(self):
        self.setup_files({'BC-GPU-001': ['core'], 'BC-GPU-002': ['core']})
        na_root = self.root / 'na-root'
        (na_root / 'results').mkdir(parents=True)
        (na_root / 'evidence').mkdir()
        evidence = na_root / 'evidence/approval.txt'
        evidence.write_text('SYNTHETIC APPROVAL ONLY')
        now = dt.datetime.now(dt.timezone.utc)
        stamp = lambda value: value.isoformat().replace('+00:00', 'Z')
        record = {'ID': 'BC-GPU-002', 'candidate_sha': 'a' * 40, 'result': 'N-A', 'owner': 'fixture',
            'evidence_link': 'synthetic', 'evidence_paths': ['evidence/approval.txt'], 'case_start_utc': stamp(now),
            'approval': {'reason': 'synthetic only', 'requester': 'requester', 'approver': 'approver',
                         'scope': 'BC-GPU-002 rehearsal', 'approved_utc': stamp(now - dt.timedelta(minutes=1)),
                         'expires_utc': stamp(now + dt.timedelta(hours=1))}}
        record_path = na_root / 'results/BC-GPU-002.json'
        record_path.write_text(json.dumps(record))
        index = json.loads(self.index.read_text())
        index['approved_na'] = [{'record': {'path': str(record_path), 'sha256': digest(record_path)},
                                  'evidence_root': str(na_root), 'evidence': {'evidence/approval.txt': digest(evidence)}}]
        def write_index():
            self.index.write_text(json.dumps(index))
        write_index()
        with patch.object(selected_set.case_launcher, 'preflight') as preflight:
            self.assertEqual([x[0] for x in self.load()], ['BC-GPU-001'])
            self.assertEqual(preflight.call_count, 1)
        # Wrong scope/candidate, expired and non-independent approvals are refused
        # even when the submitter recomputes the record pin.
        for change in ({'scope': 'different case'}, {'expires_utc': stamp(now - dt.timedelta(seconds=1))},
                       {'approver': 'REQUESTER'}):
            saved = dict(record['approval'])
            record['approval'].update(change)
            record_path.write_text(json.dumps(record))
            index['approved_na'][0]['record']['sha256'] = digest(record_path)
            write_index()
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.load()
            record['approval'] = saved
        record['candidate_sha'] = 'b' * 40
        record_path.write_text(json.dumps(record))
        index['approved_na'][0]['record']['sha256'] = digest(record_path)
        write_index()
        with self.assertRaisesRegex(ValueError, 'candidate'):
            self.load()
        record['candidate_sha'] = 'a' * 40
        record_path.write_text(json.dumps(record))
        index['approved_na'][0]['record']['sha256'] = digest(record_path)
        write_index()
        evidence.write_text('TAMPERED')
        with self.assertRaisesRegex(ValueError, 'evidence changed'):
            self.load()
        evidence.write_text('SYNTHETIC APPROVAL ONLY')
        # Duplicate/executable entries for the N/A are never accepted.
        duplicate = self.root / 'na-case.json'
        duplicate.write_text(json.dumps({'candidate_sha': 'a' * 40, 'lane': 'G2', 'cases': {'BC-GPU-002': {}}}))
        index['manifests'].append({'path': str(duplicate), 'sha256': digest(duplicate)})
        write_index()
        with patch.object(selected_set.case_launcher, 'preflight'), self.assertRaisesRegex(ValueError, 'not selected'):
            self.load()

    def test_explicit_trigger_deferral_is_an_incomplete_subset(self):
        self.catalog.write_text('ID,Tier,Parallel group\n'
            'BC-GPU-001,RC-GPU,G1\nBC-PERF-001,RC-GPU,P1\nBC-PERF-002,RC-GPU,P2\n'
            'BC-COMP-001,RC-GPU,P1\nBC-OPS-001,RC-GPU,CLEANUP\n')
        selected = {case: ['core'] for case in ('BC-GPU-001', 'BC-PERF-001', 'BC-PERF-002', 'BC-COMP-001', 'BC-OPS-001')}
        self.setup_files(selected)
        index = json.loads(self.index.read_text())
        for case, lane in (('BC-PERF-001', 'P1'), ('BC-PERF-002', 'P2')):
            path = self.root / (case + '.json')
            path.write_text(json.dumps({'candidate_sha': 'a' * 40, 'lane': lane, 'cases': {case: {}}}))
            index['manifests'].append({'path': str(path), 'sha256': digest(path)})
        index['deferred_at_trigger'] = [
            {'case': 'BC-COMP-001', 'after': ['BC-PERF-001', 'BC-PERF-002']},
            {'case': 'BC-OPS-001', 'after': ['BC-COMP-001', 'BC-GPU-001', 'BC-PERF-001', 'BC-PERF-002']},
        ]
        self.index.write_text(json.dumps(index))
        with patch('validate_results.load_catalog', return_value={}), patch('validate_results.load_selection'), \
             patch.object(selected_set.case_launcher, 'preflight') as preflight:
            with self.assertRaisesRegex(ValueError, 'explicit pretrigger'):
                selected_set.load_set(self.selection, self.catalog, self.index)
            entries = selected_set.load_set(self.selection, self.catalog, self.index, allow_pretrigger=True)
            self.assertEqual([entry[0] for entry in entries], ['BC-GPU-001', 'BC-PERF-001', 'BC-PERF-002'])
            self.assertEqual(preflight.call_count, 3)
            argv = ['selected_set', '--selection', str(self.selection), '--catalog', str(self.catalog),
                    '--index', str(self.index), '--pretrigger']
            with patch('sys.argv', argv), patch.object(selected_set.case_launcher, 'run_case') as run:
                self.assertEqual(selected_set.main(), 3)  # explicitly incomplete, not success
                self.assertEqual(run.call_count, 3)
        # No declaration can silently remove an ordinary case or alter dependencies.
        for invalid in ([{'case': 'BC-GPU-001', 'after': []}],
                        [{'case': 'BC-COMP-001', 'after': ['BC-PERF-001']}],
                        [index['deferred_at_trigger'][0], index['deferred_at_trigger'][0]],
                        [{'case': 'BC-OPS-001', 'after': ['BC-PERF-001', 'BC-PERF-002']}],
                        [{'case': [], 'after': []}]):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                selected_set.trigger_deferred_cases({'deferred_at_trigger': invalid}, set(selected), set(), True)
        with self.assertRaisesRegex(ValueError, 'both selected performance predecessors'):
            selected_set.trigger_deferred_cases({'deferred_at_trigger': [index['deferred_at_trigger'][0]]},
                                                {'BC-COMP-001', 'BC-PERF-001'}, set(), True)

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
