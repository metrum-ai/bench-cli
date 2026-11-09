#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic controls for Stage-0 milestone boundaries."""
import hashlib
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import result_writer
import stage0_milestones as checks


class MilestoneTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.exp = 'a' * 64
        self.manifest = 'b' * 64
        self.sha = 'c' * 40
        self.case = 'BC-GPU-001'
        (self.root / '.stage0-smoke-root.json').write_text(json.dumps({
            'classification': result_writer.SMOKE_CLASS, 'expectations_sha256': self.exp}))
        self.case_root = self.root / 'attempt'
        (self.case_root / 'results').mkdir(parents=True)
        (self.case_root / 'evidence').mkdir()
        self.request = self.case_root / 'evidence' / 'requests.jsonl'
        self.row = {'schema_version': 'metrum-ai-bench-cli.request.v5', 'phase': 'measure',
                    'started_at': '2026-11-09T12:00:00Z', 'completed_at': '2026-11-09T12:00:01Z',
                    'outcome': 'answer', 'answer_completed': True}
        self.write_request(self.row)
        self.record = {'ID': self.case, 'candidate_sha': self.sha, 'result': 'FAIL',
                       'evidence_class': result_writer.SMOKE_CLASS, 'stage0_expectations_sha256': self.exp,
                       'input_hashes': {'execution_manifest': self.manifest},
                       'evidence_paths': ['evidence/requests.jsonl'],
                       'output_hashes': {'evidence/requests.jsonl': self.hash(self.request)}}
        self.save_record()

    def hash(self, path):
        return hashlib.sha256(path.read_bytes()).hexdigest()

    def write_request(self, row):
        self.request.write_text(json.dumps(row) + '\n')

    def save_record(self):
        (self.case_root / 'results' / (self.case + '.json')).write_text(json.dumps(self.record))

    def check(self):
        return checks.check_record(self.case_root, case=self.case, candidate=self.sha,
                                   expectations_sha256=self.exp, manifest_sha256=self.manifest,
                                   request_path='evidence/requests.jsonl')

    def test_bound_request_and_isolated_production_rejection(self):
        with patch.object(checks.validate_results, 'check_record', wraps=checks.validate_results.check_record) as reject:
            record = self.check()
        self.assertEqual(record['result'], 'FAIL')
        reject.assert_called_once()
        self.assertEqual(list((self.case_root / 'results').iterdir()), [self.case_root / 'results' / (self.case + '.json')])

    def test_execution_index_rejects_marked_root_without_submission(self):
        with tempfile.TemporaryDirectory() as tmp:
            index = Path(tmp) / 'control.json'
            index.write_text('{}')
            kwargs = {'index_path': index, 'index_sha256': self.hash(index), 'candidate': self.sha,
                      'selection_sha256': 'd' * 64, 'catalog_sha256': 'e' * 64,
                      'expectations_sha256': self.exp}
            with patch.object(checks.execution_intake, 'ExecutionIndex',
                              wraps=checks.execution_intake.ExecutionIndex) as intake:
                checks.check_execution_intake_rejection(self.case_root, **kwargs)
            intake.assert_called_once()
            self.assertEqual(index.read_text(), '{}')
            self.assertEqual(list((self.case_root / 'results').iterdir()), [self.case_root / 'results' / (self.case + '.json')])
            with patch.object(checks.execution_intake, 'ExecutionIndex', return_value=object()):
                with self.assertRaisesRegex(ValueError, 'did not reject'):
                    checks.check_execution_intake_rejection(self.case_root, **kwargs)
            with patch.object(checks.execution_intake, 'ExecutionIndex', side_effect=ValueError('unrelated failure')):
                with self.assertRaisesRegex(ValueError, 'unrelated failure'):
                    checks.check_execution_intake_rejection(self.case_root, **kwargs)
            index.write_text('changed')
            with self.assertRaisesRegex(ValueError, 'control index'):
                checks.check_execution_intake_rejection(self.case_root, **kwargs)

    def test_rejects_unbound_or_changed_request(self):
        self.write_request({**self.row, 'outcome': 'error'})
        with self.assertRaisesRegex(ValueError, 'output hash'):
            self.check()
        self.record['output_hashes']['evidence/requests.jsonl'] = self.hash(self.request)
        self.save_record()
        with self.assertRaisesRegex(ValueError, 'no successful measured request'):
            self.check()

    def test_rejects_infra_fallback_and_mismatched_manifest(self):
        for mutation in ({'infra_history': [{'type': 'infra_interrupted'}]},
                         {'input_hashes': {'execution_manifest': 'd' * 64}}):
            with self.subTest(mutation=mutation):
                old = self.record.copy()
                self.record.update(mutation)
                self.save_record()
                with self.assertRaises(ValueError):
                    self.check()
                self.record = old
                self.save_record()

    def test_rejects_symlink_escape(self):
        self.request.unlink()
        self.request.symlink_to('/etc/hosts')
        with self.assertRaisesRegex(ValueError, 'symlinked'):
            self.check()

    def test_wrong_root_marker_and_unmarked_record(self):
        self.record['stage0_expectations_sha256'] = 'e' * 64
        self.save_record()
        with self.assertRaisesRegex(ValueError, 'not bound'):
            self.check()

    def test_server_ready_must_match_case(self):
        lifecycle = self.root / 'lifecycle'
        lifecycle.mkdir()
        (lifecycle / 'phases.jsonl').write_text(json.dumps({'case': self.case, 'phase': 'server-ready', 'utc': '2026-11-09T12:00:00Z'}) + '\n')
        checks.check_readiness(lifecycle, self.case)
        with self.assertRaisesRegex(ValueError, 'missing production server-ready'):
            checks.check_readiness(lifecycle, 'BC-GPU-002')

    def test_r4_image_and_gpu_identity_mismatch(self):
        evidence = self.case_root / 'evidence'
        (evidence / 'gpu.csv').write_text('NVIDIA H100,550.1\n')
        expected = {'image_index': 'sha256:' + '1' * 64, 'platform_manifest': 'sha256:' + '2' * 64,
                    'image_config': 'sha256:' + '3' * 64, 'platform': 'linux/amd64', 'container_verified': True,
                    'gpu_uuid': 'GPU-11111111-1111-1111-1111-111111111111',
                    'gpu_visible': {'name': 'NVIDIA H100', 'uuid': 'GPU-11111111-1111-1111-1111-111111111111', 'driver': '550.1'}}
        (evidence / 'r4-image-runtime.json').write_text(json.dumps(expected))
        kwargs = {'gpu_name': 'NVIDIA H100', 'gpu_uuid': expected['gpu_uuid'], 'index': expected['image_index'],
                  'child': expected['platform_manifest'], 'config': expected['image_config']}
        checks.check_r4_identity(self.case_root, **kwargs)
        with self.assertRaisesRegex(ValueError, 'GPU identity'):
            checks.check_r4_identity(self.case_root, **{**kwargs, 'gpu_name': 'NVIDIA RTX'})
        with self.assertRaisesRegex(ValueError, 'runtime container identity'):
            checks.check_r4_identity(self.case_root, **{**kwargs, 'config': 'sha256:' + '4' * 64})
        with self.assertRaisesRegex(ValueError, 'runtime container identity'):
            checks.check_r4_identity(self.case_root, **{**kwargs, 'gpu_uuid': 'GPU-22222222-2222-2222-2222-222222222222'})


if __name__ == '__main__':
    unittest.main()
