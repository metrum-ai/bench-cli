#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline per-box state-machine controls using a fake runtime adapter."""
import hashlib
import json
import os
import signal
import tempfile
import unittest
from pathlib import Path

import stage0_controller as controller


class FakeAdapter:
    def __init__(self):
        self.launched = []
        self.stopped = []
        self.armed = True
        self.supported = True
        self.milestones = True
        self.cleanup = True
        self.stop_verified = True
        self.outcome = {'terminal': 'COMPLETE', 'record_result': 'PASS'}
        self.on_launch = None

    def validate_box(self, _box, _cases): return self.supported
    def backstop_armed(self, _box): return self.armed
    def preflight(self, _case): return True
    def launch(self, case):
        self.launched.append(case['id'])
        if self.on_launch: self.on_launch()
        return self.outcome
    def verify_milestones(self, _case, _outcome): return self.milestones
    def verify_cleanup(self, _case): return self.cleanup
    def stop_and_verify(self, box):
        self.stopped.append(box)
        return self.stop_verified


class ControllerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / 'expectations.json'
        self.cases = ['BC-GPU-001', 'BC-GPU-002']
        self.candidate = 'a' * 40
        entries = []
        for case in self.cases:
            manifest = Path(self.temp.name) / (case + '.json')
            manifest.write_text(json.dumps({'candidate_sha': self.candidate, 'cases': {case: {}}}))
            entries.append({'id': case, 'allowed_classes': ['COMPLETE'], 'cap_seconds': 150,
                            'grace_seconds': 30, 'manifest': {'path': str(manifest),
                            'sha256': hashlib.sha256(manifest.read_bytes()).hexdigest()},
                            'request_log': 'evidence/requests.jsonl'})
        self.plan = {'candidate_sha': self.candidate, 'boxes': [{'id': 'box-a', 'cases': entries}]}
        self.path.write_text(json.dumps(self.plan))
        self.sha = hashlib.sha256(self.path.read_bytes()).hexdigest()
        self.adapter = FakeAdapter()

    def run_box(self):
        return controller.run_box(self.path, self.sha, 'box-a', self.cases, self.adapter)

    def test_per_box_barrier_after_all_milestones_and_cleanup(self):
        summary = self.run_box()
        self.assertEqual(summary['classification'], 'Stage-0 smoke, not certification')
        self.assertEqual(self.adapter.launched, self.cases)
        self.assertEqual(summary['cases'], self.cases)
        self.assertFalse(self.adapter.stopped)

    def test_missing_adapter_stops_before_first_case(self):
        self.adapter.supported = False
        with self.assertRaisesRegex(ValueError, 'validate_box'):
            self.run_box()
        self.assertFalse(self.adapter.launched)
        self.assertEqual(self.adapter.stopped, ['box-a'])

    def test_missing_milestone_stops_box_before_next_case(self):
        self.adapter.milestones = False
        with self.assertRaisesRegex(ValueError, 'verify_milestones'):
            self.run_box()
        self.assertEqual(self.adapter.launched, self.cases[:1])
        self.assertEqual(self.adapter.stopped, ['box-a'])

    def test_unexpected_pass_stops_box(self):
        self.plan['boxes'][0]['cases'][0]['allowed_classes'] = ['INCOMPLETE_GRID_FAIL']
        self.path.write_text(json.dumps(self.plan))
        self.sha = hashlib.sha256(self.path.read_bytes()).hexdigest()
        with self.assertRaisesRegex(ValueError, 'unexpected terminal'):
            self.run_box()
        self.assertEqual(self.adapter.launched, self.cases[:1])
        self.assertEqual(self.adapter.stopped, ['box-a'])

    def test_bad_class_result_pair_and_cleanup_failure_stop(self):
        self.adapter.outcome = {'terminal': 'COMPLETE', 'record_result': 'FAIL'}
        with self.assertRaisesRegex(ValueError, 'disagree'):
            self.run_box()
        self.adapter.outcome = {'terminal': 'COMPLETE', 'record_result': 'PASS'}
        self.adapter.cleanup = False
        with self.assertRaisesRegex(ValueError, 'verify_cleanup'):
            self.run_box()
        self.assertEqual(self.adapter.stopped, ['box-a', 'box-a'])

    def test_unarmed_backstop_or_changed_expectation_stops(self):
        self.adapter.armed = False
        with self.assertRaisesRegex(ValueError, 'backstop_armed'):
            self.run_box()
        self.assertFalse(self.adapter.launched)
        self.adapter.armed = True
        self.adapter.on_launch = lambda: self.path.write_text('{}')
        with self.assertRaisesRegex(ValueError, 'hash differs'):
            self.run_box()
        self.assertEqual(self.adapter.launched, self.cases[:1])
        self.assertEqual(self.adapter.stopped, ['box-a', 'box-a'])

    def test_cleanup_failure_never_reports_success(self):
        self.adapter.milestones = False
        self.adapter.stop_verified = False
        with self.assertRaisesRegex(RuntimeError, 'cleanup was not verified'):
            self.run_box()

    def test_sigterm_triggers_stop_and_restores_handler(self):
        previous = signal.getsignal(signal.SIGTERM)
        self.adapter.on_launch = lambda: os.kill(os.getpid(), signal.SIGTERM)
        with self.assertRaises(InterruptedError):
            self.run_box()
        self.assertEqual(self.adapter.stopped, ['box-a'])
        self.assertEqual(signal.getsignal(signal.SIGTERM), previous)

    def test_changed_manifest_or_invalid_cap_stops_before_launch(self):
        manifest = Path(self.plan['boxes'][0]['cases'][0]['manifest']['path'])
        manifest.write_text('{}')
        with self.assertRaisesRegex(ValueError, 'manifest hash changed'):
            self.run_box()
        self.assertFalse(self.adapter.launched)
        self.assertEqual(self.adapter.stopped, ['box-a'])
        self.plan['boxes'][0]['cases'][0]['grace_seconds'] = 150
        self.path.write_text(json.dumps(self.plan))
        self.sha = hashlib.sha256(self.path.read_bytes()).hexdigest()
        with self.assertRaisesRegex(ValueError, 'cap and cleanup grace'):
            self.run_box()
        self.assertFalse(self.adapter.launched)

    def test_duplicate_omitted_or_unassigned_case_rejected(self):
        self.plan['boxes'][0]['cases'].pop()
        self.path.write_text(json.dumps(self.plan))
        self.sha = hashlib.sha256(self.path.read_bytes()).hexdigest()
        with self.assertRaisesRegex(ValueError, 'plan differs'):
            self.run_box()
        self.assertFalse(self.adapter.launched)
        self.assertEqual(self.adapter.stopped, ['box-a'])


if __name__ == '__main__':
    unittest.main()
