#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Build measured/validation-only artifacts from one clean frozen commit."""
import argparse
import datetime as dt
import json
import os
import re
import subprocess
import tomllib
from pathlib import Path
from execution_evidence import digest, require_python


def git(repo, *args):
    return subprocess.check_output(['git', '-C', str(repo), *args], text=True).strip()


def clean_source(repo, sha):
    if not re.fullmatch(r'[a-f0-9]{40}', sha) or git(repo, 'rev-parse', 'HEAD') != sha:
        raise ValueError('build source must match the full frozen candidate commit')
    if git(repo, 'status', '--porcelain'):
        raise ValueError('build source must be clean')
    package = tomllib.loads((repo / 'Cargo.toml').read_text())
    if package.get('features', {}).get('default', []):
        raise ValueError('unexpected default features; review release equivalence')


def artifact(path, sha, role, features):
    if not path.is_file() or not os.access(path, os.X_OK):
        raise ValueError(f'missing built executable: {path.name}')
    return {'path': str(path.resolve()), 'sha256': digest(path), 'candidate_sha': sha,
            'role': role, 'features': features, 'profile': 'release'}


def build(args):
    require_python()
    repo = args.repo.resolve(strict=True)
    clean_source(repo, args.candidate_sha)
    if not re.fullmatch(r'[a-z0-9_-]+', args.rust_target) or not re.fullmatch(r'[a-z0-9_.-]+', args.zig_target):
        raise ValueError('invalid target identifier')
    if args.zig_target.split('.')[0] != args.rust_target:
        raise ValueError('zig target must use the same Rust target')
    out = args.out.resolve()
    # Build outputs cannot alter the checkout being attested as clean.
    if out == repo or repo in out.parents:
        raise ValueError('build output must be outside the frozen checkout')
    out.mkdir(parents=True, exist_ok=False)
    env = {key: os.environ[key] for key in ('PATH', 'HOME', 'CARGO_HOME', 'RUSTUP_HOME',
            'RUSTUP_TOOLCHAIN', 'TMPDIR') if key in os.environ}
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'],
                                                 cwd=repo, env=env, text=True))
    packages = [p for p in metadata['packages'] if Path(p['manifest_path']).resolve() == repo / 'Cargo.toml']
    if len(packages) != 1:
        raise ValueError('cannot identify root package')
    names = sorted(t['name'] for t in packages[0]['targets'] if 'bin' in t['kind'])
    if 'metrum-ai-bench-cli-llm' not in names:
        raise ValueError('root package lacks tokenizer validation CLI')
    result = {'_copyright': 'Copyright (c) 2026 Metrum AI, Inc.', 'candidate_sha': args.candidate_sha,
              'rust_target': args.rust_target, 'zig_target': args.zig_target,
              'started_at': dt.datetime.now(dt.timezone.utc).isoformat(), 'builds': {}}
    for role, features in [('measured', ['otlp']), ('validation-only', ['otlp', 'tokenizer'])]:
        target = out / role
        command = ['cargo', 'zigbuild', '--locked', '--release', '--features', ','.join(features),
                   '--target', args.zig_target, '--target-dir', str(target), '--package', packages[0]['name']]
        command += ['--bins'] if role == 'measured' else ['--bin', 'metrum-ai-bench-cli-llm']
        log = out / (role + '.log')
        with log.open('xb') as stream:
            subprocess.run(command, cwd=repo, env=env, stdout=stream, stderr=subprocess.STDOUT, check=True)
        clean_source(repo, args.candidate_sha)
        binaries = {name: artifact(target / args.rust_target / 'release' / name,
                                  args.candidate_sha, role, features)
                    for name in (names if role == 'measured' else ['metrum-ai-bench-cli-llm'])}
        result['builds'][role] = {'argv': command, 'log_sha256': digest(log), 'binaries': binaries}
    result['ended_at'] = dt.datetime.now(dt.timezone.utc).isoformat()
    # No completed receipt is emitted on a failed/partial build; logs remain.
    with (out / 'builds.json').open('x') as stream:
        json.dump(result, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write('\n')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('repo', 'out'):
        parser.add_argument('--' + name, type=Path, required=True)
    for name in ('candidate-sha', 'rust-target', 'zig-target'):
        parser.add_argument('--' + name, required=True)
    args = parser.parse_args()
    try:
        build(args)
    except (ValueError, KeyError, OSError, subprocess.SubprocessError) as exc:
        parser.exit(1, f'build freeze failed: {exc}\n')
    print(f'BUILD-RECEIPT {args.out / "builds.json"}; not case evidence')


if __name__ == '__main__':
    main()
