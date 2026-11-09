#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Join a pre-opened per-attempt cgroup before executing a launcher.

The supervisor must create and verify the delegated cgroup first, pass only its
procs descriptor, and independently verify its cleanup. This wrapper is not a
network namespace, sandbox, or a defence against a privileged hostile child.
"""
import os
import sys


def main(argv):
    if len(argv) < 3 or not argv[0].isdecimal() or argv[1] != '--' or not os.path.isabs(argv[2]):
        raise ValueError('expected PROCS_FD -- ABSOLUTE_EXECUTABLE [ARGS...]')
    fd = int(argv[0])
    if fd < 3:
        raise ValueError('invalid cgroup descriptor')
    data = str(os.getpid()).encode('ascii')
    try:
        if os.write(fd, data) != len(data):
            raise OSError('short cgroup join write')
    finally:
        os.close(fd)
    os.execve(argv[2], argv[2:], os.environ)


if __name__ == '__main__':
    main(sys.argv[1:])
