#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Per-attempt delegated cgroup v2 cleanup primitive, not a sandbox.

The execution host must independently prove delegation and reliable cgroup-wide
termination. No network namespace is created. A privileged or hostile child
that can escape the delegated hierarchy is outside this primitive's guarantee.
"""
import os
import re
import secrets
from pathlib import Path


_ESCAPE = re.compile(r'\\([0-7]{3})')


def require_cgroup2(root):
    """Require the nearest mount to be cgroup v2; never remount or elevate."""
    root = Path(root)
    if not root.is_absolute() or root.is_symlink() or root.resolve(strict=True) != root or not root.is_dir():
        raise ValueError('delegated cgroup root must be a canonical directory')
    mounts = []
    for line in Path('/proc/self/mountinfo').read_text().splitlines():
        left, separator, right = line.partition(' - ')
        if not separator:
            raise ValueError('invalid mount table')
        fields, after = left.split(), right.split()
        if len(fields) < 6 or not after:
            raise ValueError('invalid mount table')
        name = _ESCAPE.sub(lambda m: chr(int(m.group(1), 8)), fields[4])
        mount = Path(name)
        if root == mount or root.is_relative_to(mount):
            mounts.append((len(mount.parts), after[0]))
    if not mounts:
        raise ValueError('delegated root is not on cgroup v2')
    deepest = max(depth for depth, _kind in mounts)
    if {kind for depth, kind in mounts if depth == deepest} != {'cgroup2'}:
        raise ValueError('delegated root is not unambiguously on cgroup v2')
    if not os.access(root, os.W_OK):
        raise ValueError('cgroup root is not writable; delegation required')
    return root


class Group:
    def __init__(self, parent_fd, group_fd, join_fd, name):
        self.parent_fd, self.group_fd, self.join_fd, self.name = parent_fd, group_fd, join_fd, name
        self.closed = False

    def populated(self):
        fd = os.open('cgroup.events', os.O_RDONLY | os.O_NOFOLLOW, dir_fd=self.group_fd)
        try: raw = os.read(fd, 4097)
        finally: os.close(fd)
        if len(raw) > 4096:
            raise ValueError('cgroup events too large')
        try: lines = raw.decode('ascii').splitlines()
        except UnicodeDecodeError as exc: raise ValueError('invalid cgroup events') from exc
        values = [line.split() for line in lines if line.startswith('populated ')]
        if len(values) != 1 or len(values[0]) != 2 or values[0][1] not in ('0', '1'):
            raise ValueError('cgroup populated state is missing or invalid')
        return values[0][1] == '1'

    def kill(self):
        fd = os.open('cgroup.kill', os.O_WRONLY | os.O_NOFOLLOW, dir_fd=self.group_fd)
        try:
            if os.write(fd, b'1') != 1: raise OSError('short cgroup kill write')
        finally: os.close(fd)

    def close_if_empty(self):
        if self.closed: raise ValueError('cgroup already closed')
        if self.populated(): raise ValueError('owned cgroup remains populated')
        named = os.stat(self.name, dir_fd=self.parent_fd, follow_symlinks=False)
        opened = os.fstat(self.group_fd)
        if (named.st_dev, named.st_ino) != (opened.st_dev, opened.st_ino):
            raise ValueError('owned cgroup path changed')
        # rmdir fails closed if nested groups or other cgroup resources remain.
        os.rmdir(self.name, dir_fd=self.parent_fd)
        os.close(self.join_fd); os.close(self.group_fd); os.close(self.parent_fd)
        self.closed = True


def create(root):
    """Create a new unique child under an explicitly approved delegated root."""
    root = require_cgroup2(root)
    parent_fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    name = 'stage0-' + secrets.token_hex(16)
    group_fd = join_fd = None
    created = False
    try:
        os.mkdir(name, 0o700, dir_fd=parent_fd)
        created = True
        group_fd = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent_fd)
        join_fd = os.open('cgroup.procs', os.O_WRONLY | os.O_NOFOLLOW, dir_fd=group_fd)
        # Check that group-wide kill exists before any launcher can be attached.
        fd = os.open('cgroup.kill', os.O_WRONLY | os.O_NOFOLLOW, dir_fd=group_fd)
        os.close(fd)
        group = Group(parent_fd, group_fd, join_fd, name)
        if group.populated(): raise ValueError('new cgroup is unexpectedly populated')
        return group
    except BaseException:
        if join_fd is not None: os.close(join_fd)
        if group_fd is not None: os.close(group_fd)
        try:
            if created: os.rmdir(name, dir_fd=parent_fd)
        finally:
            os.close(parent_fd)
        raise
