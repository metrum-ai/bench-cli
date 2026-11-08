#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Freeze workflow-built measured binaries and a local validation-only CLI."""
import argparse
import datetime as dt
import json
import os
import re
import subprocess
import tomllib
from pathlib import Path
import pretag_artifact
from execution_evidence import digest, require_python


def git(repo, *args):
    return subprocess.check_output(['git', '-C', str(repo), *args], text=True).strip()


def clean_source(repo, sha, version=None):
    if not re.fullmatch(r'[a-f0-9]{40}', sha) or git(repo, 'rev-parse', 'HEAD') != sha:
        raise ValueError('build source must match the full frozen candidate commit')
    if git(repo, 'status', '--porcelain'):
        raise ValueError('build source must be clean')
    package = tomllib.loads((repo / 'Cargo.toml').read_text())
    if package.get('features', {}).get('default') != ['tokenizer']:
        raise ValueError('unexpected default features; expected shipped tokenizer profile')
    if version is not None and package['package']['version'] != version:
        raise ValueError('frozen source version must match pretag version')


def artifact(path, sha, role, features, origin):
    if not path.is_file() or not os.access(path, os.X_OK):
        raise ValueError(f'missing executable: {path.name}')
    return {'path': str(path.resolve()), 'sha256': digest(path), 'candidate_sha': sha,
            'role': role, 'features': features, 'profile': 'release', 'origin': origin}


def build(args):
    require_python()
    repo = args.repo.resolve(strict=True)
    clean_source(repo, args.candidate_sha, args.version)
    out = args.out.resolve()
    if out == repo or repo in out.parents:
        raise ValueError('build output must be outside the frozen checkout')
    out.mkdir(parents=True, exist_ok=False)
    env = {key: os.environ[key] for key in ('PATH', 'HOME', 'CARGO_HOME', 'RUSTUP_HOME',
            'RUSTUP_TOOLCHAIN', 'TMPDIR') if key in os.environ}
    result = {'_copyright': 'Copyright (c) 2026 Metrum AI, Inc.', 'candidate_sha': args.candidate_sha,
              'version': args.version, 'started_at': dt.datetime.now(dt.timezone.utc).isoformat(), 'builds': {}}
    # No local measured compiler invocation exists in this producer.
    origin, paths = pretag_artifact.download(args.candidate_sha, args.version, args.workflow_run_id, out / 'measured')
    measured = {name: artifact(path, args.candidate_sha, 'measured', ['otlp', 'tokenizer'], origin)
                for name, path in paths.items()}
    result['builds']['measured'] = {'origin': origin, 'binaries': measured}
    clean_source(repo, args.candidate_sha, args.version)
    command = ['cargo', 'build', '--locked', '--release', '--features', 'otlp,tokenizer',
               '--target-dir', str(out / 'validation-only'), '--bin', 'metrum-ai-bench-cli-llm']
    log = out / 'validation-only.log'
    with log.open('xb') as stream:
        subprocess.run(command, cwd=repo, env=env, stdout=stream, stderr=subprocess.STDOUT, check=True)
    clean_source(repo, args.candidate_sha, args.version)
    helper = artifact(out / 'validation-only' / 'release' / 'metrum-ai-bench-cli-llm',
                      args.candidate_sha, 'validation-only', ['otlp', 'tokenizer'],
                      {'kind': 'local-cargo-validation'})
    result['builds']['validation-only'] = {'argv': command, 'log_sha256': digest(log),
                                         'binaries': {'metrum-ai-bench-cli-llm': helper}}
    result['ended_at'] = dt.datetime.now(dt.timezone.utc).isoformat()
    # Partial downloads/compiler logs survive failure; never emit partial receipt.
    with (out / 'builds.json').open('x') as stream:
        json.dump(result, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write('\n')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('repo', 'out'):
        parser.add_argument('--' + name, type=Path, required=True)
    for name in ('candidate-sha', 'version'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--workflow-run-id', type=int, required=True)
    args = parser.parse_args()
    try:
        build(args)
    except (ValueError, KeyError, OSError, subprocess.SubprocessError) as exc:
        parser.exit(1, f'build freeze failed: {exc}\n')
    print(f'BUILD-RECEIPT {args.out / "builds.json"}; not case evidence')


if __name__ == '__main__':
    main()
