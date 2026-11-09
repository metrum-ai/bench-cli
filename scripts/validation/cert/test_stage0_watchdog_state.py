#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline durable-state protocol tests; no provider request."""
import json
import tempfile
import unittest
from pathlib import Path

from stage0_backstop import BackstopScope
from stage0_watchdog_state import State


class StateTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.scope = BackstopScope('bench-cli-v190-stage0-' + 'a' * 24, 100.0)
        self.state = State(self.root, self.scope)

    def put(self, name, data):
        self.state._write_once(name, {'name': self.scope.name, **data})

    def test_exclusive_durable_arm_and_cleanup_receipts(self):
        self.state.armed(self.scope)
        self.assertEqual(json.loads((self.root / 'armed.json').read_text())['deadline_epoch'], 100)
        with self.assertRaises(FileExistsError): self.state.armed(self.scope)
        self.state.cleaned(self.scope)
        self.assertTrue(json.loads((self.root / 'cleaned.json').read_text())['clean'])

    def test_creation_must_settle_and_match_identity(self):
        self.assertFalse(self.state.creation_closed())
        self.assertIsNone(self.state.resource_id())
        self.put('resource.json', {'id': 'id_123'})
        self.put('creation-closed.json', {'id': 'different', 'settled': True})
        with self.assertRaisesRegex(ValueError, 'differ'): self.state.creation_closed()
        (self.root / 'creation-closed.json').unlink()
        self.put('creation-closed.json', {'id': 'id_123', 'settled': True})
        self.assertTrue(self.state.creation_closed())

    def test_stop_invalid_markers_and_symlinks_fail_closed(self):
        self.assertFalse(self.state.stop_requested())
        self.put('stop.json', {'stop': True})
        self.assertTrue(self.state.stop_requested())
        (self.root / 'stop.json').unlink()
        (self.root / 'stop.json').symlink_to('/etc/hosts')
        with self.assertRaises(OSError): self.state.stop_requested()

    def test_duplicate_json_and_wrong_scope_fail(self):
        path = self.root / 'resource.json'
        path.write_text('{"name":"x","name":"y"}'); path.chmod(0o600)
        with self.assertRaisesRegex(ValueError, 'duplicate'): self.state.resource_id()
        path.write_text(json.dumps({'name': 'wrong', 'id': 'id'}))
        with self.assertRaisesRegex(ValueError, 'scope'): self.state.resource_id()

    def test_fake_provider_watchdog_observes_settled_create_before_clean(self):
        from stage0_watchdog import watch
        pods = []
        class Provider:
            def inventory(_self): return list(pods)
            def delete(_self, ident): pods[:] = [p for p in pods if p['id'] != ident]
        now = [1]
        def pause(seconds):
            now[0] += seconds
            if not pods and not (self.root / 'resource.json').exists():
                pods.append({'id': 'owned', 'name': self.scope.name})
                self.put('resource.json', {'id': 'owned'})
                self.put('creation-closed.json', {'id': 'owned', 'settled': True})
        watch(self.scope, Provider(), self.state, clock=lambda: now[0], pause=pause, interval=30)
        self.assertTrue((self.root / 'armed.json').is_file())
        self.assertTrue((self.root / 'cleaned.json').is_file())
        self.assertFalse(pods)

    def test_reports_only_exception_class(self):
        self.state.error('TimeoutError')
        self.assertEqual((self.root / 'errors.log').read_text(), 'TimeoutError\n')
        with self.assertRaises(ValueError): self.state.error('message with secret')


if __name__ == '__main__': unittest.main()
