#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline fake-provider controls; never makes a provider request."""
import unittest

import stage0_backstop as backstop


class BackstopTests(unittest.TestCase):
    def setUp(self):
        self.name = 'bench-cli-v190-stage0-' + 'a' * 24
        self.scope = backstop.BackstopScope(self.name, 100)
        self.resources = []
        self.deleted = []

    def inventory(self):
        return list(self.resources)

    def delete(self, resource_id):
        self.deleted.append(resource_id)
        self.resources = [r for r in self.resources if r['id'] != resource_id]

    def enforce(self, **kwargs):
        args = {'resource_id': 'owned-id', 'creation_closed': True, 'stop_requested': False, 'now': 100}
        args.update(kwargs)
        return backstop.enforce(self.scope, self.inventory, self.delete, **args)

    def test_arm_fails_closed_on_collision_or_expiry(self):
        backstop.check_available(self.scope, self.inventory, 10)
        self.resources = [{'id': 'owned-id', 'name': self.name}]
        with self.assertRaisesRegex(ValueError, 'already exists'):
            backstop.check_available(self.scope, self.inventory, 10)
        with self.assertRaisesRegex(ValueError, 'elapsed'):
            backstop.check_available(self.scope, lambda: [], 100)

    def test_deadline_deletes_only_exact_name_and_id_then_absence(self):
        self.resources = [{'id': 'unrelated', 'name': self.name + '-other'}, {'id': 'owned-id', 'name': self.name}]
        self.assertFalse(self.enforce())
        self.assertEqual(self.deleted, ['owned-id'])
        self.assertTrue(self.enforce())
        self.assertTrue(self.enforce())  # separate second absent provider inventory
        self.assertEqual(self.resources, [{'id': 'unrelated', 'name': self.name + '-other'}])

    def test_wrong_id_or_ambiguous_scope_never_deleted(self):
        self.resources = [{'id': 'other-id', 'name': self.name}]
        with self.assertRaisesRegex(ValueError, 'name/ID mismatch'):
            self.enforce()
        self.resources.append({'id': 'second', 'name': self.name})
        with self.assertRaisesRegex(ValueError, 'ambiguous'):
            self.enforce()
        self.assertFalse(self.deleted)

    def test_inflight_create_cannot_be_declared_clean(self):
        self.assertFalse(self.enforce(creation_closed=False))
        self.resources = [{'id': 'owned-id', 'name': self.name}]
        self.assertFalse(self.enforce(creation_closed=False))
        self.assertEqual(self.deleted, ['owned-id'])
        self.assertFalse(self.enforce(creation_closed=False))
        self.assertTrue(self.enforce(creation_closed=True))

    def test_early_teardown_must_be_explicit(self):
        with self.assertRaisesRegex(ValueError, 'explicit stop'):
            self.enforce(now=10)
        self.assertTrue(self.enforce(now=10, stop_requested=True))

    def test_bad_scope_rejected(self):
        for name in ('x', self.name.upper(), 'bench-cli-v190-stage0-../../oops'):
            with self.subTest(name=name), self.assertRaises(ValueError):
                backstop.BackstopScope(name, 100)


if __name__ == '__main__':
    unittest.main()
