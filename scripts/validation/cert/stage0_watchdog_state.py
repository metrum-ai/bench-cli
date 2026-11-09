#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Watchdog state protocol prototype, with no provider requests.

State files coordinate a separately deployed watchdog and controller. File
presence is not proof of process health or a separate failure domain. The
controller may close creation only after a known terminal provider outcome;
an ambiguous timed-out create must stay open until independently resolved.
"""
import json
import os
import re
import stat
from pathlib import Path

NAME = re.compile(r'bench-cli-v190-stage0-[a-f0-9]{24}\Z')
ID = re.compile(r'[A-Za-z0-9_-]{1,128}\Z')


class State:
    def __init__(self, root, scope):
        self.root = Path(root).resolve(strict=True)
        if (not self.root.is_dir() or self.root.stat().st_uid != os.geteuid()
                or self.root.stat().st_mode & 0o077 or not NAME.fullmatch(scope.name)):
            raise ValueError('watchdog state must be owner-only and uniquely scoped')
        self.scope = scope

    def _read(self, name):
        path = self.root / name
        try:
            fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        except FileNotFoundError:
            return None
        try:
            meta = os.fstat(fd)
            if not stat.S_ISREG(meta.st_mode) or meta.st_uid != os.geteuid() or meta.st_mode & 0o077 or meta.st_size > 4096:
                raise ValueError('invalid watchdog state file')
            raw = os.read(fd, 4097)
        finally:
            os.close(fd)
        if len(raw) > 4096:
            raise ValueError('watchdog state file too large')
        def unique(pairs):
            output = {}
            for key, value in pairs:
                if key in output: raise ValueError('duplicate state key')
                output[key] = value
            return output
        data = json.loads(raw, object_pairs_hook=unique)
        if not isinstance(data, dict) or data.get('name') != self.scope.name:
            raise ValueError('watchdog state scope mismatch')
        return data

    def _write_once(self, name, data):
        # Write/flush the entire record before publishing it as a hard link;
        # existing receipts cannot be silently replaced.
        temp = self.root / ('.tmp-' + os.urandom(16).hex())
        fd = os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        try:
            with os.fdopen(fd, 'w', encoding='utf-8') as stream:
                json.dump(data, stream, sort_keys=True, allow_nan=False)
                stream.write('\n'); stream.flush(); os.fsync(stream.fileno())
            os.link(temp, self.root / name, follow_symlinks=False)
            dfd = os.open(self.root, os.O_RDONLY | os.O_DIRECTORY)
            try: os.fsync(dfd)
            finally: os.close(dfd)
        finally:
            temp.unlink(missing_ok=True)

    def armed(self, scope):
        if scope != self.scope: raise ValueError('watchdog scope mismatch')
        self._write_once('armed.json', {'name': scope.name, 'deadline_epoch': scope.deadline_epoch,
                                        'watchdog_pid': os.getpid()})

    def stop_requested(self):
        data = self._read('stop.json')
        if data is None: return False
        if set(data) != {'name', 'stop'} or data['stop'] is not True:
            raise ValueError('invalid stop request')
        return True

    def resource_id(self):
        data = self._read('resource.json')
        if data is None: return None
        ident = data.get('id')
        if set(data) != {'name', 'id'} or not isinstance(ident, str) or not ID.fullmatch(ident):
            raise ValueError('invalid provider identity')
        return ident

    def creation_closed(self):
        data = self._read('creation-closed.json')
        if data is None: return False
        if set(data) != {'name', 'settled', 'id'} or data['settled'] is not True:
            raise ValueError('creation not known to be settled')
        ident = self.resource_id()
        if data['id'] != ident:
            raise ValueError('creation closure and resource identity differ')
        return True

    def cleaned(self, scope):
        if scope != self.scope: raise ValueError('watchdog scope mismatch')
        self._write_once('cleaned.json', {'name': scope.name, 'deadline_epoch': scope.deadline_epoch,
                                          'clean': True})

    def error(self, kind):
        if not re.fullmatch(r'[A-Za-z][A-Za-z0-9_]{0,63}', kind):
            raise ValueError('invalid error class')
        # Report only error class; never provider bodies, tokens or URLs.
        path = self.root / 'errors.log'
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND | os.O_NOFOLLOW | os.O_NONBLOCK, 0o600)
        with os.fdopen(fd, 'a', encoding='ascii') as stream:
            meta = os.fstat(stream.fileno())
            if not stat.S_ISREG(meta.st_mode) or meta.st_uid != os.geteuid() or meta.st_mode & 0o077:
                raise ValueError('invalid watchdog error log')
            if meta.st_size < 1_048_576:
                stream.write(kind + '\n'); stream.flush(); os.fsync(stream.fileno())
