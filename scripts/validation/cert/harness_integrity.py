#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Check the local harness source tree against a specified candidate commit.

This is a source-byte check, not an attestation of Git, the host, Python startup,
bytecode caches, shared libraries, builds, or the validator's execution context.
"""
import hashlib
import re
import subprocess
from pathlib import Path

DIR_SCOPES = ('scripts/validation/cert', 'scripts/validation/perf', 'xtask/src')
FILE_SCOPES = ('Cargo.toml', 'Cargo.lock', 'xtask/Cargo.toml', 'xtask/Cargo.lock',
               'xtask/build.rs', 'build.rs', 'src/count_points.rs',
               'scripts/check_release_readme_links.py', 'test-data/vlm/shapes-512.png', '.cargo/config.toml', '.cargo/config',
               'rust-toolchain', 'rust-toolchain.toml')
SCOPES = DIR_SCOPES + FILE_SCOPES
CODE_SUFFIXES = {'.py', '.sh', '.json', '.rs', '.toml', '.lock'}


def git(repo, *args, input=None):
    # Do not inherit caller-selected git config, replacements or path.
    env = {'PATH': '/usr/bin:/bin', 'HOME': '/nonexistent', 'LC_ALL': 'C',
           'GIT_CONFIG_NOSYSTEM': '1', 'GIT_CONFIG_GLOBAL': '/dev/null',
           'GIT_NO_REPLACE_OBJECTS': '1'}
    try:
        return subprocess.run(['/usr/bin/git', '-C', str(repo), *args], input=input,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                              check=True, timeout=60, env=env).stdout
    except subprocess.SubprocessError as exc:
        raise ValueError('candidate Git source verification failed') from exc


def relative_pins(repo, pins):
    """Canonical relative source identities, portable between reviewed checkouts."""
    if not isinstance(repo, str):
        raise ValueError('harness repository path must be a string')
    root = Path(repo)
    if not root.is_absolute() or str(root) != repo or '..' in root.parts:
        raise ValueError('harness repository path must be absolute and normalized')
    if not isinstance(pins, list):
        raise ValueError('harness closure must be a list')
    result = {}
    for item in pins:
        if not isinstance(item, dict) or set(item) != {'path', 'sha256'}:
            raise ValueError('invalid harness source pin')
        path, value = item['path'], item['sha256']
        if (not isinstance(path, str) or not Path(path).is_absolute() or str(Path(path)) != path
                or '..' in Path(path).parts or not isinstance(value, str)
                or not re.fullmatch(r'[0-9a-f]{64}', value)):
            raise ValueError('invalid harness source path or hash')
        try:
            name = Path(path).relative_to(root).as_posix()
        except ValueError as exc:
            raise ValueError('harness source outside repository') from exc
        if name in result:
            raise ValueError('duplicate harness source')
        result[name] = value
    return result


def verify(repo, candidate):
    repo = Path(repo).resolve(strict=True)
    if not isinstance(candidate, str) or not re.fullmatch(r'[0-9a-f]{40}', candidate):
        raise ValueError('candidate must be a full SHA-1 commit')
    if git(repo, 'cat-file', '-t', candidate).strip() != b'commit':
        raise ValueError('candidate is not a commit')
    tree = git(repo, 'ls-tree', '-r', '-z', '--full-tree', candidate, '--', *SCOPES)
    tracked = {}
    for item in tree.split(b'\0'):
        if not item:
            continue
        info, raw_path = item.split(b'\t', 1)
        mode, kind, oid = info.split(b' ', 2)
        path = raw_path.decode('utf-8')
        if mode not in (b'100644', b'100755') or kind != b'blob':
            raise ValueError('harness closure contains a symlink or non-file')
        tracked[path] = (mode, oid)
    if not {'scripts/validation/cert/case_launcher.py', 'scripts/validation/cert/validate_results.py',
            'scripts/validation/cert/result_writer.py', 'xtask/src/main.rs', 'Cargo.toml',
            'Cargo.lock', 'xtask/Cargo.toml', 'src/count_points.rs', '.cargo/config.toml',
            'scripts/validation/perf/runner.sh', 'scripts/check_release_readme_links.py',
            'test-data/vlm/shapes-512.png'} <= tracked.keys():
        raise ValueError('candidate lacks mandatory harness closure')
    for name in FILE_SCOPES:
        path = repo / name
        if path.is_symlink() or (path.exists() and name not in tracked):
            raise ValueError('untracked or unsafe harness build configuration')
    for directory in DIR_SCOPES:
        for path in (repo / directory).rglob('*'):
            if '__pycache__' in path.parts:
                continue  # bytecode startup is a separate unresolved host boundary
            if path.is_symlink():
                raise ValueError('harness closure contains a local symlink')
            if path.is_file() and path.suffix in CODE_SUFFIXES and path.relative_to(repo).as_posix() not in tracked:
                raise ValueError('untracked executable or configuration in harness closure')
    # Batch-read the candidate's actual Git objects, rather than comparing
    # caller-supplied hashes or trusting an index/worktree diff.
    ordered = sorted(tracked)
    objects = git(repo, 'cat-file', '--batch', input=b''.join(tracked[p][1] + b'\n' for p in ordered))
    pos = 0
    pins = []
    for name in ordered:
        end = objects.find(b'\n', pos)
        if end < 0:
            raise ValueError('truncated Git object stream')
        header = objects[pos:end].split(b' ')
        if len(header) != 3 or header[0] != tracked[name][1] or header[1] != b'blob':
            raise ValueError('invalid Git object stream')
        size = int(header[2])
        start = end + 1
        blob = objects[start:start + size]
        if len(blob) != size or objects[start + size:start + size + 1] != b'\n':
            raise ValueError('truncated Git blob')
        pos = start + size + 1
        path = repo / name
        if any((repo / Path(*Path(name).parts[:i])).is_symlink() for i in range(1, len(Path(name).parts) + 1)):
            raise ValueError(f'harness source traverses a symlink: {name}')
        if not path.is_file() or not path.resolve().is_relative_to(repo):
            raise ValueError(f'missing or unsafe harness source: {name}')
        if bool(path.stat().st_mode & 0o111) != (tracked[name][0] == b'100755'):
            raise ValueError(f'harness executable mode differs: {name}')
        actual = hashlib.sha256(path.read_bytes()).digest()
        if actual != hashlib.sha256(blob).digest():
            raise ValueError(f'harness source differs from candidate: {name}')
        pins.append({'path': str(path), 'sha256': actual.hex()})
    if pos != len(objects):
        raise ValueError('trailing Git object data')
    return pins
