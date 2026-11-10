#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Scoped container reconciliation primitives; not a real Docker adapter."""
import os
import re
import stat
from pathlib import Path

OWNER = re.compile(r'[0-9a-f]{32}\Z')
CONTAINER = re.compile(r'[0-9a-f]{12,64}\Z')


def read_owner(root, role):
    if role not in ('server', 'auxiliary'):
        raise ValueError('invalid ownership role')
    path = Path(root) / 'evidence' / (role + '-container-owner.txt')
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        meta = os.fstat(fd)
        if not stat.S_ISREG(meta.st_mode) or stat.S_IMODE(meta.st_mode) != 0o600 or meta.st_size != 33:
            raise ValueError('invalid ownership receipt')
        raw = os.read(fd, 34)
    finally:
        os.close(fd)
    try:
        text = raw.decode('ascii')
    except UnicodeDecodeError as exc:
        raise ValueError('invalid ownership receipt') from exc
    if not text.endswith('\n') or not OWNER.fullmatch(text[:-1]):
        raise ValueError('invalid ownership receipt')
    return text[:-1]


def reconcile(owner, adapter, *, creation_closed):
    """Remove only exact-owner resources; leaked resources always fail the case.

    Adapter inventory(owner) returns container IDs; inspect_owner(id) returns the
    independently inspected label; remove(id) stops and removes exactly that ID.
    Return 'clean' only for two empty inventories after creation is closed. If a
    leak had to be removed, return 'leak_removed' even after confirmed absence.
    Any ambiguity or provider failure must raise and stop the box. A production
    caller must supply a separately verified pinned Docker adapter.
    """
    if not isinstance(owner, str) or not OWNER.fullmatch(owner) or type(creation_closed) is not bool:
        raise ValueError('invalid cleanup scope')

    def inventory():
        values = adapter.inventory(owner)
        if (not isinstance(values, list) or any(not isinstance(x, str) or not CONTAINER.fullmatch(x) for x in values)
                or len(values) != len(set(values))):
            raise ValueError('invalid container inventory')
        return values

    found = inventory()
    for ident in found:
        if adapter.inspect_owner(ident) != owner:
            raise ValueError('container owner mismatch; refusing removal')
    for ident in found:
        adapter.remove(ident)
    if inventory() or inventory():
        raise ValueError('owned container remains')
    if not creation_closed:
        raise ValueError('container creation is not closed')
    return 'leak_removed' if found else 'clean'
