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
        self.manifest.write_text(json.dumps({'candidate_sha': self.sha, 'cases': {self.case: {'out': str(self.smoke / 'attempt')}},
            'tools': {'python3': {'path': '/usr/bin/python3', 'sha256': 'c' * 64},
                      'docker': {'path': '/usr/bin/docker', 'sha256': 'd' * 64}}}))
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
                        patch.object(stage0_attempt.stage0_docker, 'Docker')]
        self.mocks = [p.start() for p in self.patches]
        for p in self.patches: self.addCleanup(p.stop)

    def run_attempt(self, **overrides):
        args = {'candidate': self.sha, 'expectations_sha256': self.exp, 'smoke_root': self.smoke,
                'cgroup_root': '/approved/cgroup', 'forwarded': {'CUDA_VISIBLE_DEVICES': '0'}}
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
        with self.assertRaisesRegex(RuntimeError, 'cleanup was not verified'):
            self.run_attempt()
        self.assertEqual(self.mocks[4].call_count, 2)

    def test_unapproved_environment_rejected_before_launch(self):
        with self.assertRaisesRegex(ValueError, 'environment'):
            self.run_attempt(forwarded={'CUDA_VISIBLE_DEVICES': '0', 'PATH': '/bad'})
        self.mocks[2].assert_not_called()


if __name__ == '__main__': unittest.main()
