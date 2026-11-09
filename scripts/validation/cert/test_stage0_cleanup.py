#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline fake-container reconciliation checks."""
import tempfile
import unittest
from pathlib import Path

import stage0_cleanup as cleanup

OWNER = 'a' * 32
CONTAINER = 'b' * 64


class FakeDocker:
    def __init__(self, items=None):
        self.items = list(items or [])
        self.deleted = []
        self.label = OWNER
        self.calls = 0
    def inventory(self, _owner):
        self.calls += 1
        return list(self.items)
    def inspect_owner(self, _id): return self.label
    def remove(self, ident):
        self.deleted.append(ident)
        self.items.remove(ident)


class CleanupTests(unittest.TestCase):
    def test_clean_requires_independent_absent_inventories_and_closed_create(self):
        docker = FakeDocker()
        self.assertEqual(cleanup.reconcile(OWNER, docker, creation_closed=True), 'clean')
        self.assertEqual(docker.calls, 3)
        with self.assertRaisesRegex(ValueError, 'not closed'):
            cleanup.reconcile(OWNER, docker, creation_closed=False)

    def test_leak_is_removed_but_never_classed_clean(self):
        docker = FakeDocker([CONTAINER])
        self.assertEqual(cleanup.reconcile(OWNER, docker, creation_closed=True), 'leak_removed')
        self.assertEqual(docker.deleted, [CONTAINER])
        self.assertFalse(docker.items)

    def test_mismatch_and_ambiguous_inventory_never_remove(self):
        docker = FakeDocker([CONTAINER]); docker.label = 'x' * 32
        with self.assertRaisesRegex(ValueError, 'mismatch'): cleanup.reconcile(OWNER, docker, creation_closed=True)
        self.assertFalse(docker.deleted)
        for ids in ([CONTAINER, CONTAINER], ['invalid; shell'], [1]):
            docker = FakeDocker(ids)
            with self.subTest(ids=ids), self.assertRaises(ValueError):
                cleanup.reconcile(OWNER, docker, creation_closed=True)
            self.assertFalse(docker.deleted)

    def test_continued_presence_fails_closed(self):
        docker = FakeDocker([CONTAINER]); docker.remove = lambda _: None
        with self.assertRaisesRegex(ValueError, 'remains'):
            cleanup.reconcile(OWNER, docker, creation_closed=True)

    def test_receipt_must_be_private_regular_and_not_symlink(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); (root / 'evidence').mkdir()
            path = root / 'evidence/server-container-owner.txt'
            path.write_text(OWNER + '\n'); path.chmod(0o600)
            self.assertEqual(cleanup.read_owner(root, 'server'), OWNER)
            path.chmod(0o644)
            with self.assertRaises(ValueError): cleanup.read_owner(root, 'server')
            path.unlink(); path.symlink_to(root / 'target')
            with self.assertRaises(OSError): cleanup.read_owner(root, 'server')


if __name__ == '__main__': unittest.main()
