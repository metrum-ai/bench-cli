#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Fake-backed tests of the narrow GPU-001 adapter; no GPU or provider work."""
import hashlib
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import MagicMock, patch

import stage0_controller
import stage0_gpu001_adapter
import stage0_process


class AdapterTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name); self.sha = 'a' * 40; self.case = 'BC-GPU-001'
        self.out = self.root / 'smoke' / 'attempt'; self.out.parent.mkdir()
        self.manifest = self.root / 'case.json'
        self.manifest.write_text(json.dumps({'candidate_sha': self.sha, 'cases': {self.case: {'out': str(self.out)}}}))
        self.item = {'id': self.case, 'allowed_classes': ['COMPLETE'], 'cap_seconds': 150, 'grace_seconds': 30,
                     'manifest': {'path': str(self.manifest), 'sha256': hashlib.sha256(self.manifest.read_bytes()).hexdigest()},
                     'request_log': 'evidence/g1-clock-default-requests.jsonl'}
        self.plan = self.root / 'expectations.json'
        self.plan_value = {'candidate_sha': self.sha, 'smoke_root': str(self.out.parent),
                           'selection_sha256': 'c' * 64, 'catalog_sha256': 'd' * 64,
                           'boxes': [{'id': 'box-a', 'gpu': 'NVIDIA H100', 'cgroup_root': '/cgroup', 'cases': [self.item]}]}
        self.plan.write_text(json.dumps(self.plan_value))
        self.exp = hashlib.sha256(self.plan.read_bytes()).hexdigest()
        self.stop = MagicMock(return_value=True)
        self.adapter = stage0_gpu001_adapter.Adapter(box='box-a', gpu='NVIDIA H100', candidate=self.sha,
            expectations_sha256=self.exp, smoke_root=self.out.parent, cgroup_root='/cgroup',
            forwarded={'CUDA_VISIBLE_DEVICES': '0'}, control_index=self.root / 'control.json',
            control_index_sha256='b' * 64, selection_sha256='c' * 64, catalog_sha256='d' * 64,
            backstop_probe=lambda _: True, stop_and_verify=self.stop)
        def run(*_args, **_kwargs):
            (self.out / 'results').mkdir(parents=True)
            (self.out / 'results' / (self.case + '.json')).write_text(json.dumps({'result': 'PASS'}))
            return stage0_process.ProcessOutcome(0, False, False)
        patches = [patch.object(stage0_gpu001_adapter.stage0_attempt, 'run', side_effect=run),
                   patch.object(stage0_gpu001_adapter.stage0_milestones, 'check_r4_identity'),
                   patch.object(stage0_gpu001_adapter.stage0_milestones, 'check_readiness'),
                   patch.object(stage0_gpu001_adapter.stage0_milestones, 'check_record', return_value={'result': 'PASS'}),
                   patch.object(stage0_gpu001_adapter.stage0_milestones, 'check_execution_intake_rejection')]
        self.mocks = [p.start() for p in patches]
        for p in patches: self.addCleanup(p.stop)

    def run_box(self):
        return stage0_controller.run_box(self.plan, self.exp, 'box-a', [self.case], self.adapter)

    def test_all_five_checks_and_cleanup_before_summary(self):
        result = self.run_box()
        self.assertEqual(result['classification'], 'Stage-0 smoke, not certification')
        for mock in self.mocks: mock.assert_called_once()
        self.stop.assert_not_called()

    def test_wrong_gpu_stops_box(self):
        self.mocks[1].side_effect = ValueError('wrong GPU')
        with self.assertRaisesRegex(ValueError, 'wrong GPU'): self.run_box()
        self.stop.assert_called_once_with('box-a')

    def test_box_identity_mismatch_never_launches(self):
        self.adapter.gpu = 'NVIDIA RTX'
        with self.assertRaisesRegex(ValueError, 'validate_box'): self.run_box()
        self.mocks[0].assert_not_called()
        self.stop.assert_called_once_with('box-a')

    def test_unsupported_terminal_or_case_never_launches(self):
        self.item['allowed_classes'] = ['INCOMPLETE_GRID_FAIL']
        self.plan.write_text(json.dumps(self.plan_value))
        self.exp = hashlib.sha256(self.plan.read_bytes()).hexdigest()
        self.adapter.expectations_sha256 = self.exp
        with self.assertRaisesRegex(ValueError, 'validate_box'): self.run_box()
        self.mocks[0].assert_not_called()
        self.stop.assert_called_once_with('box-a')


if __name__ == '__main__': unittest.main()
