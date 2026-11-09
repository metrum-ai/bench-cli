#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline fake-backed generic common-server adapter controls."""
import hashlib
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import MagicMock, patch

import r4_model_pins
import stage0_controller
import stage0_dispatch
import stage0_generic_adapter
import stage0_process


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class AdapterTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name); self.sha = 'a' * 40
        self.smoke = self.root / 'smoke'; self.smoke.mkdir()
        self.items = [self.case('BC-GPU-001', 'G1'), self.case('BC-GPU-004', 'G2')]
        catalog = digest(stage0_dispatch.REPO / 'docs/RELEASE_CERTIFICATION_CATALOG.csv')
        self.plan = self.root / 'expectations.json'
        self.plan_value = {'candidate_sha': self.sha, 'smoke_root': str(self.smoke),
                           'selection_sha256': 'c' * 64, 'catalog_sha256': catalog,
                           'boxes': [{'id': 'box-a', 'gpu': 'NVIDIA H100', 'cgroup_root': '/cgroup', 'cases': self.items}]}
        self.save_plan()
        self.stop = MagicMock(return_value=True)
        self.adapter = stage0_generic_adapter.Adapter(box='box-a', gpu='NVIDIA H100', candidate=self.sha,
            expectations_sha256=self.exp, smoke_root=self.smoke, cgroup_root='/cgroup',
            forwarded={'CUDA_VISIBLE_DEVICES': 'GPU-11111111-1111-1111-1111-111111111111'}, control_index=self.root / 'control.json',
            control_index_sha256='b' * 64, selection_sha256='c' * 64, catalog_sha256=catalog,
            backstop_probe=lambda _: True, stop_and_verify=self.stop)
        def run(item, **_kwargs):
            manifest = stage0_controller.pinned_manifest(item, self.sha)
            output = Path(manifest['cases'][item['id']]['out'])
            (output / 'results').mkdir(parents=True)
            (output / 'results' / (item['id'] + '.json')).write_text(json.dumps({'result': 'PASS'}))
            return stage0_process.ProcessOutcome(0, False, False)
        patches = [patch.object(stage0_generic_adapter.stage0_attempt, 'run', side_effect=run),
                   patch.object(stage0_generic_adapter.stage0_milestones, 'check_r4_identity'),
                   patch.object(stage0_generic_adapter.stage0_milestones, 'check_readiness'),
                   patch.object(stage0_generic_adapter.stage0_milestones, 'check_record', return_value={'result': 'PASS'}),
                   patch.object(stage0_generic_adapter.stage0_milestones, 'check_execution_intake_rejection'),
                   patch.object(stage0_generic_adapter.stage0_milestones, 'check_server_evidence')]
        self.mocks = [p.start() for p in patches]
        for p in patches: self.addCleanup(p.stop)

    def case(self, case, lane):
        lane_file = self.root / (case + '-lane.json')
        lane_file.write_text(json.dumps({'lanes': {lane: {'model': r4_model_pins.GENERAL_MODEL,
            'revision': r4_model_pins.GENERAL_REVISION, 'image': r4_model_pins.IMAGE}}}))
        out = str(self.smoke / case); launcher = str(stage0_dispatch.HERE / 'case_entry.sh')
        manifest = {'candidate_sha': self.sha, 'lane': lane, 'harness_repo': str(stage0_dispatch.REPO),
            'candidate': {'path': '/candidate'}, 'baseline': {'path': '/baseline'},
            'lane_manifest': {'path': str(lane_file), 'sha256': digest(lane_file)},
            'cases': {case: {'out': out, 'launcher': {'path': launcher},
                      'command': [launcher, '--case', case, '--lane', lane.lower(), '--candidate', '/candidate',
                                  '--baseline', '/baseline', '--manifest', str(lane_file), '--out', out]}}}
        path = self.root / (case + '.json'); path.write_text(json.dumps(manifest))
        return {'id': case, 'allowed_classes': ['COMPLETE'], 'cap_seconds': 150, 'grace_seconds': 30,
                'manifest': {'path': str(path), 'sha256': digest(path)}, 'request_log': 'evidence/requests.jsonl'}

    def save_plan(self):
        self.plan.write_text(json.dumps(self.plan_value)); self.exp = digest(self.plan)
        if hasattr(self, 'adapter'): self.adapter.expectations_sha256 = self.exp

    def run_box(self):
        return stage0_controller.run_box(self.plan, self.exp, 'box-a', [item['id'] for item in self.items], self.adapter)

    def test_two_lanes_share_generic_adapter(self):
        result = self.run_box()
        self.assertEqual(result['classification'], 'Stage-0 smoke, not certification')
        self.assertEqual(result['cases'], ['BC-GPU-001', 'BC-GPU-004'])
        for mock in self.mocks: self.assertEqual(mock.call_count, 2)
        self.stop.assert_not_called()

    def test_l2_modality_uses_generic_milestones(self):
        self.items.append(self.case('BC-LIVE-002', 'L2')); self.save_plan()
        result = self.run_box()
        self.assertEqual(result['cases'][-1], 'BC-LIVE-002')
        self.assertEqual(self.mocks[0].call_count, 3)
        self.assertEqual(self.mocks[5].call_count, 3)

    def test_wrong_gpu_stops_before_next_case(self):
        self.mocks[1].side_effect = ValueError('wrong GPU')
        with self.assertRaisesRegex(ValueError, 'wrong GPU'): self.run_box()
        self.mocks[0].assert_called_once(); self.stop.assert_called_once_with('box-a')

    def test_runtime_generated_manifest_must_bind_approved_parent(self):
        item = self.items[0]; manifest_path = Path(item['manifest']['path'])
        manifest = json.loads(manifest_path.read_text()); manifest['generated_inputs'] = [{'fixture': True}]
        manifest_path.write_text(json.dumps(manifest)); item['manifest']['sha256'] = digest(manifest_path)
        output = Path(manifest['cases'][item['id']]['out'])
        generated = output.with_name(output.name + '-lifecycle') / 'generated'; generated.mkdir(parents=True)
        runtime = {'candidate_sha': self.sha, 'parent_structural_manifest': dict(item['manifest'])}
        path = generated / 'case.json'; path.write_text(json.dumps(runtime))
        with patch('runtime_inputs.verify') as verify:
            self.assertEqual(self.adapter.executed_manifest_sha(item, output), digest(path))
            verify.assert_called_once_with(runtime)
            runtime['parent_structural_manifest']['sha256'] = 'e' * 64; path.write_text(json.dumps(runtime))
            with self.assertRaisesRegex(ValueError, 'approved structural'):
                self.adapter.executed_manifest_sha(item, output)

    def test_box_identity_mismatch_never_launches(self):
        self.adapter.gpu = 'NVIDIA RTX'
        with self.assertRaisesRegex(ValueError, 'validate_box'): self.run_box()
        self.mocks[0].assert_not_called(); self.stop.assert_called_once_with('box-a')

    def test_unsupported_class_or_override_blocks_entire_box(self):
        self.items[1]['allowed_classes'] = ['INCOMPLETE_GRID_FAIL']; self.save_plan()
        with self.assertRaisesRegex(ValueError, 'validate_box'): self.run_box()
        self.mocks[0].assert_not_called()
        self.items[1] = self.case('BC-PERF-001', 'P1'); self.save_plan()
        with self.assertRaisesRegex(ValueError, 'override'): self.run_box()
        self.mocks[0].assert_not_called()


if __name__ == '__main__': unittest.main()
