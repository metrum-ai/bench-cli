# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic final-intake controls; never evidence of an actual GPU launch."""
import copy
import datetime as dt
import json
import tempfile
import unittest
from pathlib import Path

import execution_intake
import select_cases
import validate_results as validator


class ExecutionIntakeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.results = self.root / 'results'
        self.results.mkdir()
        self.proofs = self.root / 'proofs'
        self.proofs.mkdir()
        self.catalog = self.root / 'catalog.csv'
        self.catalog.write_text('ID,Tier,Test case,Selection,Area,Parallel group\n'
            'BC-GPU-001,RC-GPU,GPU fixture,core,GPU,G1\n'
            'BC-INTEG-001,RC-GPU,NA fixture,core,INTEG,W\n')
        self.sha = 'a' * 40
        self.selection_path = self.root / 'selection.json'
        self.selection = select_cases.make_selection(validator.load_catalog(self.catalog),
            execution_intake.digest(self.catalog), self.sha, .1, 3, [], [])
        self.selection_path.write_text(json.dumps(self.selection))
        self.now = dt.datetime(2026, 11, 9, tzinfo=dt.timezone.utc)
        self.evidence = self.proofs / 'evidence.txt'
        self.evidence.write_text('SYNTHETIC ONLY\n')
        self.tools = {name: {'path': '/synthetic/' + name, 'sha256': 'b' * 64}
                      for name in ('bash', 'env', 'python3')}
        self.manifest_path = self.proofs / 'manifest.json'
        self.manifest = {'candidate_sha': self.sha, 'lane': 'G1', 'cases': {'BC-GPU-001': {}}, 'tools': self.tools}
        self.manifest_path.write_text(json.dumps(self.manifest))
        self.phases_path = self.proofs / 'phases.jsonl'
        self.phases_path.write_text('\n'.join(json.dumps({'case': 'BC-GPU-001', 'phase': phase, 'utc': utc})
            for phase, utc in (('inputs-ready', '2026-11-08T11:59:00Z'),
                               ('record-validated', '2026-11-08T12:01:01Z'))) + '\n')
        self.record_path = self.results / 'BC-GPU-001.json'
        self.record = {'ID': 'BC-GPU-001', 'candidate_sha': self.sha, 'result': 'PASS',
            'owner': 'fixture', 'evidence_link': 'synthetic', 'attempt': 1,
            'observed_values': {'value': 1}, 'thresholds': {'min': 1},
            'start_utc': '2026-11-08T12:00:00Z', 'end_utc': '2026-11-08T12:01:00Z',
            'evidence_paths': ['proofs/evidence.txt'],
            'output_hashes': {'proofs/evidence.txt': execution_intake.digest(self.evidence)},
            'input_hashes': {'execution_manifest': execution_intake.digest(self.manifest_path)},
            'versions': {'execution_tools': self.tools}}
        self.record_path.write_text(json.dumps(self.record))
        self.na = {'ID': 'BC-INTEG-001', 'candidate_sha': self.sha, 'result': 'N-A', 'owner': 'fixture',
            'evidence_link': 'synthetic', 'evidence_paths': ['proofs/evidence.txt'],
            'case_start_utc': '2026-11-08T12:00:00Z',
            'approval': {'reason': 'fixture', 'requester': 'requester', 'approver': 'approver',
                         'scope': 'fixture', 'approved_utc': '2026-11-08T11:00:00Z', 'expires_utc': '2026-11-10T00:00:00Z'}}
        (self.results / 'BC-INTEG-001.json').write_text(json.dumps(self.na))
        self.plan_path = self.proofs / 'selected-plan.json'
        self.plan = {'candidate_sha': self.sha, 'selection_sha256': execution_intake.digest(self.selection_path),
                     'catalog_sha256': execution_intake.digest(self.catalog),
                     'manifests': [{'path': '/synthetic/original', 'sha256': execution_intake.digest(self.manifest_path)}]}
        self.plan_path.write_text(json.dumps(self.plan))
        self.index_path = self.root / 'execution-index.json'
        self.index = {'candidate_sha': self.sha, 'selection_sha256': execution_intake.digest(self.selection_path),
            'catalog_sha256': execution_intake.digest(self.catalog),
            'selected_plan': {'path': 'proofs/selected-plan.json', 'sha256': execution_intake.digest(self.plan_path)}, 'entries': [{
                'ID': 'BC-GPU-001', 'record_sha256': execution_intake.digest(self.record_path),
                'manifest': {'path': 'proofs/manifest.json', 'sha256': execution_intake.digest(self.manifest_path)},
                'phases': {'path': 'proofs/phases.jsonl', 'sha256': execution_intake.digest(self.phases_path)}}]}
        self.write_index()

    def write_index(self):
        self.index_path.write_text(json.dumps(self.index))

    def validate(self, use_index=True):
        kwargs = {'selection': self.selection}
        if use_index:
            kwargs.update(execution_index=self.index_path, execution_index_sha256=execution_intake.digest(self.index_path),
                          selection_sha256=execution_intake.digest(self.selection_path))
        return validator.validate(self.catalog, self.results, self.sha, self.now, **kwargs)

    def test_fresh_pass_requires_bound_index_while_na_remains_na(self):
        table, errors = self.validate(False)
        self.assertTrue(any('execution index' in e for e in errors))
        self.assertEqual('N-A', next(r['Result'] for r in table if r['ID'] == 'BC-INTEG-001'))
        table, errors = self.validate()
        self.assertEqual([], errors)
        self.assertEqual({'PASS', 'N-A'}, {r['Result'] for r in table})

    def test_rejects_record_or_proof_changes(self):
        for path in (self.record_path, self.manifest_path, self.phases_path, self.evidence):
            before = path.read_bytes()
            with self.subTest(path=path.name):
                path.write_bytes(before + b' ')
                self.assertTrue(self.validate()[1])
                path.write_bytes(before)

    def test_fresh_non_gpu_selected_pass_also_requires_index(self):
        self.catalog.write_text(self.catalog.read_text().replace('BC-GPU-001,RC-GPU,', 'BC-GPU-001,RC,'))
        self.selection = select_cases.make_selection(validator.load_catalog(self.catalog),
            execution_intake.digest(self.catalog), self.sha, .1, 3, [], [])
        self.assertTrue(any('execution index' in error for error in self.validate(False)[1]))

    def test_rehashed_wrong_lane_is_rejected(self):
        self.manifest['lane'] = 'G2'
        self.manifest_path.write_text(json.dumps(self.manifest))
        changed = execution_intake.digest(self.manifest_path)
        self.plan['manifests'][0]['sha256'] = changed
        self.plan_path.write_text(json.dumps(self.plan))
        self.index['selected_plan']['sha256'] = execution_intake.digest(self.plan_path)
        self.index['entries'][0]['manifest']['sha256'] = changed
        self.record['input_hashes']['execution_manifest'] = changed
        self.record_path.write_text(json.dumps(self.record))
        self.index['entries'][0]['record_sha256'] = execution_intake.digest(self.record_path)
        self.write_index()
        self.assertTrue(any('lane' in error for error in self.validate()[1]))

    def test_manifest_must_be_in_frozen_plan(self):
        self.plan['manifests'][0]['sha256'] = 'c' * 64
        self.plan_path.write_text(json.dumps(self.plan))
        self.index['selected_plan']['sha256'] = execution_intake.digest(self.plan_path)
        self.write_index()
        self.assertTrue(any('selected plan' in e for e in self.validate()[1]))

    def test_rehashed_manifest_does_not_match_record(self):
        self.manifest['tools']['python3']['sha256'] = 'c' * 64
        self.manifest_path.write_text(json.dumps(self.manifest))
        self.index['entries'][0]['manifest']['sha256'] = execution_intake.digest(self.manifest_path)
        self.write_index()
        self.assertTrue(any('manifest' in e for e in self.validate()[1]))

    def test_runtime_manifest_requires_structural_parent_and_fixed_identity(self):
        structural = {**self.manifest, 'cases': {'BC-GPU-001': {'command': ['/synthetic/launcher', '--manifest', '/synthetic/lane']}},
                      'inputs': [], 'lane_manifest': {'path': '/synthetic/lane', 'sha256': 'd' * 64},
                      'generated_inputs': [{'synthetic': True}]}
        structural_path = self.proofs / 'structural.json'
        structural_path.write_text(json.dumps(structural))
        structural_sha = execution_intake.digest(structural_path)
        runtime = {**structural, 'parent_structural_manifest': {'path': '/synthetic/structural.json', 'sha256': structural_sha},
                   'generated_bindings': {'synthetic': True}}
        self.manifest_path.write_text(json.dumps(runtime))
        self.plan['manifests'][0]['sha256'] = structural_sha
        self.plan_path.write_text(json.dumps(self.plan))
        self.index['selected_plan']['sha256'] = execution_intake.digest(self.plan_path)
        item = self.index['entries'][0]
        item['structural_manifest'] = {'path': 'proofs/structural.json', 'sha256': structural_sha}
        item['manifest']['sha256'] = execution_intake.digest(self.manifest_path)
        self.record['input_hashes']['execution_manifest'] = item['manifest']['sha256']
        self.record_path.write_text(json.dumps(self.record))
        item['record_sha256'] = execution_intake.digest(self.record_path)
        self.write_index()
        self.assertEqual([], self.validate()[1])
        runtime['candidate_sha'] = 'c' * 40
        self.manifest_path.write_text(json.dumps(runtime))
        item['manifest']['sha256'] = execution_intake.digest(self.manifest_path)
        self.record['input_hashes']['execution_manifest'] = item['manifest']['sha256']
        self.record_path.write_text(json.dumps(self.record))
        item['record_sha256'] = execution_intake.digest(self.record_path)
        self.write_index()
        self.assertTrue(any('candidate' in error for error in self.validate()[1]))

    def test_rejects_wrong_candidate_and_digest(self):
        self.index['candidate_sha'] = 'c' * 40
        self.write_index()
        with self.assertRaisesRegex(ValueError, 'candidate'):
            self.validate()
        with self.assertRaisesRegex(ValueError, 'hash mismatch'):
            execution_intake.ExecutionIndex(self.index_path, 'f' * 64, self.root, self.sha,
                execution_intake.digest(self.selection_path), execution_intake.digest(self.catalog))

    def test_rejects_incomplete_lifecycle_even_when_rehashed(self):
        self.phases_path.write_text(json.dumps({'case': 'BC-GPU-001', 'phase': 'inputs-ready'}) + '\n')
        self.index['entries'][0]['phases']['sha256'] = execution_intake.digest(self.phases_path)
        self.write_index()
        self.assertTrue(any('lifecycle' in e for e in self.validate()[1]))

    def test_rejects_extra_duplicate_and_escaping_entries(self):
        entry = self.index['entries'][0]
        self.index['entries'].append(copy.deepcopy(entry))
        self.write_index()
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            self.validate()
        self.index['entries'].pop()
        entry['manifest']['path'] = '../outside'
        self.write_index()
        self.assertTrue(any('path' in e for e in self.validate()[1]))

    def test_expired_na_is_not_laundered_by_valid_index(self):
        self.na['approval']['expires_utc'] = '2026-11-08T13:00:00Z'
        (self.results / 'BC-INTEG-001.json').write_text(json.dumps(self.na))
        self.assertTrue(any('unexpired' in e for e in self.validate()[1]))


if __name__ == '__main__':
    unittest.main()
