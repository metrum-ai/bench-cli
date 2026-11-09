#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline fake-provider and fake-clock tests for the independent watchdog."""
import unittest

from stage0_backstop import BackstopScope
from stage0_watchdog import watch


class FakeProvider:
    def __init__(self):
        self.resources = []
        self.deletions = []
        self.inventories = 0
        self.fail_next = False

    def inventory(self):
        self.inventories += 1
        if self.fail_next:
            self.fail_next = False
            raise OSError('do not log provider response')
        return list(self.resources)

    def delete(self, ident):
        self.deletions.append(ident)
        self.resources = [x for x in self.resources if x['id'] != ident]


class FakeState:
    def __init__(self, provider):
        self.provider = provider
        self.armed_count = 0
        self.cleaned_count = 0
        self.errors = []
        self.closed = True
        self.stop = False
        self.name = None

    def armed(self, scope):
        self.name = scope.name
        self.armed_count += 1
        self.provider.resources.append({'name': scope.name, 'id': 'owned-id'})

    def creation_closed(self): return self.closed
    def stop_requested(self): return self.stop
    def resource_id(self): return 'owned-id'
    def cleaned(self, _scope): self.cleaned_count += 1
    def error(self, kind): self.errors.append(kind)


class WatchdogTests(unittest.TestCase):
    def setUp(self):
        self.scope = BackstopScope('bench-cli-v190-stage0-' + 'a' * 24, 10)
        self.provider = FakeProvider()
        self.state = FakeState(self.provider)
        self.now = 1

    def clock(self): return self.now
    def pause(self, seconds): self.now += seconds

    def test_deadline_deletes_and_confirms_two_absent_inventories(self):
        watch(self.scope, self.provider, self.state, clock=self.clock, pause=self.pause, interval=3)
        self.assertEqual(self.state.armed_count, 1)
        self.assertEqual(self.provider.deletions, ['owned-id'])
        self.assertEqual(self.state.cleaned_count, 1)
        self.assertGreaterEqual(self.provider.inventories, 4)  # arm, delete, two absent
        self.assertGreaterEqual(self.now, self.scope.deadline_epoch)

    def test_error_retries_without_marking_clean(self):
        def pause(seconds):
            self.now += seconds
            if self.now >= 10 and not self.state.errors:
                self.provider.fail_next = True
        watch(self.scope, self.provider, self.state, clock=self.clock, pause=pause, interval=3)
        self.assertEqual(self.state.errors, ['OSError'])
        self.assertEqual(self.provider.deletions, ['owned-id'])
        self.assertEqual(self.state.cleaned_count, 1)

    def test_early_stop_requires_signal(self):
        self.state.stop = True
        watch(self.scope, self.provider, self.state, clock=self.clock, pause=self.pause, interval=1)
        self.assertLess(self.now, self.scope.deadline_epoch)
        self.assertEqual(self.provider.deletions, ['owned-id'])

    def test_creation_must_close_before_clean(self):
        self.state.closed = False
        def pause(seconds):
            self.now += seconds
            if self.now > 20:
                raise RuntimeError('test stops an intentionally incomplete create')
        with self.assertRaisesRegex(RuntimeError, 'intentionally incomplete'):
            watch(self.scope, self.provider, self.state, clock=self.clock, pause=pause, interval=3)
        self.assertEqual(self.state.cleaned_count, 0)
        self.assertEqual(self.provider.deletions, ['owned-id'])

    def test_collision_blocks_arm(self):
        self.provider.resources = [{'name': self.scope.name, 'id': 'other-id'}]
        with self.assertRaisesRegex(ValueError, 'already exists'):
            watch(self.scope, self.provider, self.state, clock=self.clock, pause=self.pause, interval=3)
        self.assertEqual(self.state.armed_count, 0)
        self.assertEqual(self.provider.deletions, [])


if __name__ == '__main__':
    unittest.main()
