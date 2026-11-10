#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Fail-closed case tool pins and isolated child environment.

Tools are explicit absolute, content-pinned executables. The private PATH
contains only absolute symlinks to those pins, never caller PATH entries. This
controls executable selection, not the security of the host or dynamic loader.
"""
import hashlib
import json
import os
import re
from pathlib import Path

# Operational values used by the lane code; credentials are not forwarded.
FORWARDED = frozenset({
    'CUDA_VISIBLE_DEVICES', 'CERT_ATTEMPT', 'CERT_HF_HUB_CACHE',
    'CARGO_HOME', 'CARGO_TARGET_DIR',
    'RUSTUP_HOME', 'RUSTUP_TOOLCHAIN', 'TMPDIR',
})
NAME = re.compile(r'[A-Za-z0-9][A-Za-z0-9_.+-]*\Z')
SHA = re.compile(r'[0-9a-f]{64}\Z')
REQUIRED = frozenset({'bash', 'env', 'python3'})
CASE_TOOLS = frozenset({'nvidia-smi', 'docker', 'curl', 'sha256sum', 'cut', 'date',
                        'dirname', 'mkdir', 'realpath', 'grep', 'sleep'})
CARGO_LANES = frozenset({'L1', 'L2', 'L3', 'L4', 'G6', 'P1', 'P2'})


def file_hash(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()


def validate_tools(tools):
    if not isinstance(tools, dict) or not REQUIRED <= tools.keys():
        raise ValueError('pinned tool map must include bash, env and python3')
    result = {}
    for name, item in tools.items():
        if not isinstance(name, str) or not NAME.fullmatch(name):
            raise ValueError('invalid pinned tool name')
        if not isinstance(item, dict) or set(item) != {'path', 'sha256'}:
            raise ValueError('pinned tool requires exactly path and sha256')
        path, expected = item['path'], item['sha256']
        if not isinstance(path, str) or not Path(path).is_absolute() or not isinstance(expected, str) or not SHA.fullmatch(expected):
            raise ValueError('invalid pinned tool path or sha256')
        resolved = Path(path).resolve(strict=True)
        if str(resolved) != path or not resolved.is_file() or not os.access(resolved, os.X_OK):
            raise ValueError('pinned tool must be a canonical executable file')
        with resolved.open('rb') as stream:
            if stream.read(4) != b'\x7fELF':
                raise ValueError(f'pinned tool must be a native ELF executable, not a script shim: {name}')
        if file_hash(resolved) != expected:
            raise ValueError(f'pinned tool changed: {name}')
        result[name] = {'path': str(resolved), 'sha256': expected}
    return result


def validate_case_tools(tools, lane):
    tools = validate_tools(tools)
    required = REQUIRED | (CASE_TOOLS if lane not in {'CLEANUP', 'KVV', 'KVS', 'KVL', 'KVD', 'KVT', 'KVC'} else set())
    if lane in CARGO_LANES:
        required |= {'cargo', 'rustc', 'git'}
    missing = required - tools.keys()
    if missing:
        raise ValueError(f'case is missing pinned tools: {sorted(missing)}')
    # Scripts use /usr/bin/env in their kernel shebang, independent of PATH.
    if tools['env']['path'] != str(Path('/usr/bin/env').resolve(strict=True)):
        raise ValueError('env pin must match the shebang interpreter')
    return tools


def validate_launcher(path, tools):
    """Require a pinned interpreter for a directly executed script."""
    with Path(path).open('rb') as stream:
        first = stream.readline(4097)
    if not first.startswith(b'#!'):
        return  # A separately pinned native executable.
    if len(first) > 4096:
        raise ValueError('launcher shebang is too long')
    try:
        words = first[2:].decode('utf-8').strip().split()
    except UnicodeDecodeError as exc:
        raise ValueError('invalid launcher shebang') from exc
    if not words or not Path(words[0]).is_absolute():
        raise ValueError('launcher shebang requires an absolute interpreter')
    interpreter = str(Path(words[0]).resolve(strict=True))
    if interpreter == tools['env']['path']:
        if len(words) != 2 or words[1] not in tools:
            raise ValueError('env launcher requires one explicitly pinned interpreter')
    elif len(words) > 2 or not any(item['path'] == interpreter for item in tools.values()):
        raise ValueError('launcher shebang interpreter is not pinned')


def prepare(tools, root, parent):
    tools = validate_tools(tools)
    root = Path(root).resolve(strict=True)
    bindir = root / 'tool-bin'
    bindir.mkdir(mode=0o700)
    home = root / 'home'
    home.mkdir(mode=0o700)
    for name, item in tools.items():
        (bindir / name).symlink_to(item['path'])
    verify(tools, bindir)
    env = {key: parent[key] for key in FORWARDED if key in parent}
    env.update({'PATH': str(bindir), 'HOME': str(home), 'PYTHONNOUSERSITE': '1', 'CERT_OFFLINE': '1',
                'NO_PROXY': 'localhost,127.0.0.1,::1', 'no_proxy': 'localhost,127.0.0.1,::1'})
    if 'rustc' in tools:
        env['RUSTC'] = tools['rustc']['path']
    if 'rustdoc' in tools:
        env['RUSTDOC'] = tools['rustdoc']['path']
    return env


def verify(tools, bindir):
    tools = validate_tools(tools)
    bindir = Path(bindir)
    if set(p.name for p in bindir.iterdir()) != set(tools):
        raise ValueError('pinned tool directory changed')
    for name, item in tools.items():
        link = bindir / name
        if not link.is_symlink() or os.readlink(link) != item['path']:
            raise ValueError(f'pinned tool link changed: {name}')


def load_manifest(path):
    # Duplicate keys must never silently replace a tool pin.
    from manifest_tool import unique_object
    data = json.loads(Path(path).read_text(), object_pairs_hook=unique_object)
    return validate_tools(data)
