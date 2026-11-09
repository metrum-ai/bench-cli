#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline orchestration controls with fake cgroup, Docker and launcher."""
import hashlib
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import MagicMock, patch

import result_writer
import stage0_attempt
import stage0_process


class AttemptTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name); self.smoke = self.root / 'smoke'; self.smoke.mkdir()
        self.sha = 'a' * 40; self.exp = 'b' * 64; self.case = 'BC-GPU-001'
        (self.smoke / '.stage0-smoke-root.json').write_text(json.dumps({
            'classification': result_writer.SMOKE_CLASS, 'expectations_sha256': self.exp}))
        self.manifest = self.root / 'manifest.json'
        entrypoint = str(stage0_attempt.HERE / 'case_entry.sh')
        output = str(self.smoke / 'attempt')
        self.data = {'candidate_sha': self.sha, 'lane': 'G1', 'harness_repo': str(stage0_attempt.HERE.parents[2]),
            'candidate': {'path': '/candidate'}, 'baseline': {'path': '/baseline'}, 'lane_manifest': {'path': '/lane'},
            'cases': {self.case: {'out': output, 'launcher': {'path': entrypoint},
                'command': [entrypoint, '--case', self.case, '--lane', 'g1', '--candidate', '/candidate',
                            '--baseline', '/baseline', '--manifest', '/lane', '--out', output]}},
            'tools': {'python3': {'path': '/usr/bin/python3', 'sha256': 'c' * 64},
                      'docker': {'path': '/usr/bin/docker', 'sha256': 'd' * 64}}}
        self.manifest.write_text(json.dumps(self.data))
        self.item = {'id': self.case, 'manifest': {'path': str(self.manifest),
                     'sha256': hashlib.sha256(self.manifest.read_bytes()).hexdigest()},
                     'cap_seconds': 150, 'grace_seconds': 30}
        self.group = MagicMock(name='group')
        self.group.name = 'stage0-' + 'e' * 32; self.group.join_fd = 10
        self.group.populated.return_value = False
        self.outcome = stage0_process.ProcessOutcome(0, False, False)
        self.patches = [patch.object(stage0_attempt.case_launcher, 'preflight'),
                        patch.object(stage0_attempt.stage0_cgroup, 'create', return_value=self.group),
                        patch.object(stage0_attempt.stage0_process, 'run_bounded', return_value=self.outcome),
                        patch.object(stage0_attempt.stage0_cleanup, 'read_owner', return_value='f' * 32),
                        patch.object(stage0_attempt.stage0_cleanup, 'reconcile', return_value='clean'),
                        patch.object(stage0_attempt.stage0_docker, 'Docker'),
                        patch.object(stage0_attempt.stage0_device, 'probe', return_value={
                            'name': 'NVIDIA H100', 'uuid': 'GPU-' + '1' * 32, 'driver': '550.1'}),
                        patch.object(stage0_attempt.stage0_settlement, 'successful_gpu001_create', return_value=True)]
        self.mocks = [p.start() for p in self.patches]
        for p in self.patches: self.addCleanup(p.stop)

    def run_attempt(self, **overrides):
        args = {'candidate': self.sha, 'expectations_sha256': self.exp, 'smoke_root': self.smoke,
                'cgroup_root': '/approved/cgroup', 'expected_gpu': 'NVIDIA H100',
                'forwarded': {'CUDA_VISIBLE_DEVICES': 'GPU-11111111-1111-1111-1111-111111111111'}}
        args.update(overrides)
        return stage0_attempt.run(self.item, **args)

    def test_scoped_attempt_executes_with_join_fd_and_reconciliation(self):
        self.assertEqual(self.run_attempt(), self.outcome)
        launch = self.mocks[2].call_args
        self.assertEqual(launch.kwargs['pass_fds'], (10,))
        self.assertEqual(launch.kwargs['env']['PATH'], '/nonexistent')
        self.mocks[4].assert_called_once()
        self.group.close_if_empty.assert_called_once()
        receipt = json.loads((self.smoke / 'attempt-stage0-control/cgroup.json').read_text())
        self.assertEqual(receipt['name'], self.group.name)

    def test_process_leak_is_killed_and_container_reconciliation_continues(self):
        self.group.populated.side_effect = [True, False, False]
        with self.assertRaisesRegex(RuntimeError, 'owned process survived'):
            self.run_attempt()
        self.group.kill.assert_called_once()
        self.mocks[4].assert_called_once()

    def test_interruption_fails_closed_after_cleanup_attempt(self):
        self.mocks[2].side_effect = InterruptedError('stop')
        with self.assertRaisesRegex(RuntimeError, 'settlement is unproven') as raised:
            self.run_attempt()
        self.assertIsInstance(raised.exception.__cause__, InterruptedError)
        self.group.close_if_empty.assert_called_once()
        self.mocks[4].assert_called_once()
        self.assertFalse(self.mocks[4].call_args.kwargs['creation_closed'])

    def test_timeout_never_assumes_docker_creation_has_settled(self):
        self.mocks[2].return_value = stage0_process.ProcessOutcome(-15, True, False)
        with self.assertRaisesRegex(RuntimeError, 'settlement is unproven'):
            self.run_attempt()
        self.assertFalse(self.mocks[4].call_args.kwargs['creation_closed'])

    def test_cleanup_uncertainty_never_returns_success(self):
        self.mocks[4].return_value = 'leak_removed'
        with self.assertRaisesRegex(RuntimeError, 'container survived'):
            self.run_attempt()

    def test_all_container_roles_attempted_after_one_cleanup_failure(self):
        self.case = 'BC-PERF-001'
        self.manifest.write_text(json.dumps({'candidate_sha': self.sha,
            'cases': {self.case: {'out': str(self.smoke / 'attempt')}},
            'tools': {'python3': {'path': '/usr/bin/python3', 'sha256': 'c' * 64},
                      'docker': {'path': '/usr/bin/docker', 'sha256': 'd' * 64}}}))
        self.item['id'] = self.case
        self.item['manifest']['sha256'] = hashlib.sha256(self.manifest.read_bytes()).hexdigest()
        self.mocks[4].side_effect = [ValueError('bad server'), 'clean']
        with patch.object(stage0_attempt, 'validate_dispatch'), self.assertRaisesRegex(RuntimeError, 'cleanup was not verified'):
            self.run_attempt()
        self.assertEqual(self.mocks[4].call_count, 2)

    def test_successful_exit_without_create_proof_still_reconciles_and_stops(self):
        self.mocks[7].side_effect = ValueError('missing create proof')
        with self.assertRaisesRegex(RuntimeError, 'settlement is unproven'): self.run_attempt()
        self.mocks[4].assert_called_once()
        self.assertFalse(self.mocks[4].call_args.kwargs['creation_closed'])

    def test_unreviewed_dispatch_never_launches(self):
        self.data['cases'][self.case]['command'].append('--extra')
        self.manifest.write_text(json.dumps(self.data))
        self.item['manifest']['sha256'] = hashlib.sha256(self.manifest.read_bytes()).hexdigest()
        with self.assertRaisesRegex(ValueError, 'unsupported smoke dispatch'): self.run_attempt()
        self.mocks[2].assert_not_called()
        self.group.close_if_empty.assert_not_called()

    def test_malformed_launcher_outcome_does_not_skip_container_cleanup(self):
        self.mocks[2].return_value = object()
        with self.assertRaisesRegex(RuntimeError, 'settlement is unproven'):
            self.run_attempt()
        self.assertFalse(self.mocks[4].call_args.kwargs['creation_closed'])

    def test_wrong_gpu_refuses_launch_and_changed_gpu_stops(self):
        self.mocks[6].side_effect = ValueError('wrong GPU')
        with self.assertRaisesRegex(ValueError, 'wrong GPU'): self.run_attempt()
        self.mocks[2].assert_not_called()
        before = {'name': 'NVIDIA H100', 'uuid': 'GPU-' + '1' * 32, 'driver': '550.1'}
        self.mocks[6].side_effect = [before, {**before, 'uuid': 'GPU-' + '2' * 32}]
        with self.assertRaisesRegex(RuntimeError, 'cleanup was not verified'):
            self.run_attempt()
        self.mocks[4].assert_called_once()

    def test_unapproved_environment_rejected_before_launch(self):
        with self.assertRaisesRegex(ValueError, 'environment'):
            self.run_attempt(forwarded={'CUDA_VISIBLE_DEVICES': 'GPU-11111111-1111-1111-1111-111111111111', 'PATH': '/bad'})
        self.mocks[2].assert_not_called()


if __name__ == '__main__': unittest.main()
