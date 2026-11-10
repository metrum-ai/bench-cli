#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Pinned local Docker adapter for per-attempt labelled-container reconciliation.

Inventory/removal only, no create. Reconciliation must first bind its owner to an
actual pre-create receipt. This adapter alone does not prove process cleanup.
"""
import hashlib
import math
import os
import re
import subprocess
import time
from pathlib import Path

OWNER = re.compile(r'[0-9a-f]{32}\Z')
ID = re.compile(r'[0-9a-f]{12,64}\Z')
SHA = re.compile(r'[0-9a-f]{64}\Z')


class Docker:
    def __init__(self, pin, home, *, owner, deadline=None, clock=time.monotonic, invoke=subprocess.run):
        if not isinstance(pin, dict) or set(pin) != {'path', 'sha256'}:
            raise ValueError('Docker executable pin is required')
        if (not isinstance(pin['path'], str) or not Path(pin['path']).is_absolute()
                or not isinstance(pin['sha256'], str) or not SHA.fullmatch(pin['sha256'])):
            raise ValueError('invalid Docker executable pin')
        if not isinstance(owner, str) or not OWNER.fullmatch(owner):
            raise ValueError('invalid owner')
        if deadline is not None and (type(deadline) not in (int, float) or not math.isfinite(deadline)):
            raise ValueError('invalid cleanup deadline')
        self.deadline, self.clock = deadline, clock
        self.owner = owner
        self.pin = dict(pin)
        self.home = Path(home).resolve(strict=True)
        if (not self.home.is_dir() or self.home.stat().st_uid != os.geteuid()
                or self.home.stat().st_mode & 0o077):
            raise ValueError('private Docker home must be owner-only')
        self.config = self.home / 'stage0-docker-config'
        self.invoke = invoke
        self._verify()
        self.config.mkdir(mode=0o700)

    def _verify(self):
        path = Path(self.pin['path'])
        if path.resolve(strict=True) != path or not path.is_file() or not os.access(path, os.X_OK):
            raise ValueError('Docker executable is not canonical')
        h = hashlib.sha256()
        with path.open('rb') as stream:
            if stream.read(4) != b'\x7fELF':
                raise ValueError('Docker executable must be native ELF')
            stream.seek(0)
            for chunk in iter(lambda: stream.read(1024 * 1024), b''):
                h.update(chunk)
        if h.hexdigest() != self.pin['sha256']:
            raise ValueError('Docker executable changed')

    def _run(self, args):
        self._verify()
        if (self.home.stat().st_uid != os.geteuid() or self.home.stat().st_mode & 0o077
                or self.config.is_symlink() or not self.config.is_dir()
                or self.config.stat().st_uid != os.geteuid() or self.config.stat().st_mode & 0o077
                or any(self.config.iterdir())):
            raise ValueError('private Docker config changed')
        def checked_clock():
            value = self.clock()
            if type(value) not in (int, float) or not math.isfinite(value):
                raise ValueError('invalid cleanup clock')
            return value
        timeout = 15.0
        if self.deadline is not None:
            timeout = min(timeout, self.deadline - checked_clock())
            if timeout <= 0:
                raise ValueError('Docker cleanup deadline elapsed')
        result = self.invoke([self.pin['path'], '--host', 'unix:///var/run/docker.sock', *args],
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            env={'PATH': '/nonexistent', 'HOME': str(self.home), 'DOCKER_CONFIG': str(self.config)},
            timeout=timeout, check=False)
        if self.deadline is not None and checked_clock() > self.deadline:
            raise ValueError('Docker cleanup deadline elapsed')
        if result.returncode != 0 or not isinstance(result.stdout, bytes) or len(result.stdout) > 65536:
            raise ValueError('Docker operation failed or output invalid')
        try:
            return result.stdout.decode('ascii')
        except UnicodeDecodeError as exc:
            raise ValueError('Docker output is not ASCII') from exc

    def inventory(self, owner):
        if owner != self.owner:
            raise ValueError('inventory owner differs from bound owner')
        output = self._run(['ps', '-a', '--no-trunc', '--filter', 'label=metrum.cert.owner=' + owner,
                            '--format', '{{.ID}}'])
        values = output.splitlines()
        if any(not ID.fullmatch(item) for item in values) or len(values) != len(set(values)):
            raise ValueError('invalid Docker inventory')
        return values

    def inspect_owner(self, ident):
        if not isinstance(ident, str) or not ID.fullmatch(ident):
            raise ValueError('invalid container ID')
        output = self._run(['inspect', '--format', '{{index .Config.Labels "metrum.cert.owner"}}', ident])
        if not output.endswith('\n') or not OWNER.fullmatch(output[:-1]):
            raise ValueError('Docker owner label missing or invalid')
        return output[:-1]

    def remove(self, ident):
        if not isinstance(ident, str) or not ID.fullmatch(ident):
            raise ValueError('invalid container ID')
        if self.inspect_owner(ident) != self.owner:
            raise ValueError('container owner mismatch; refusing removal')
        self._run(['rm', '-f', ident])
