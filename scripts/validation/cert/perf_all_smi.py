# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Verify the all-smi spec source and reproduce its pinned executable offline."""
import hashlib
import io
import os
from pathlib import Path
import platform
import shutil
import signal
import subprocess
import tarfile
import tempfile

from manifest_tool import sha256_file

SOURCE_REVISION = '3b6eaec3364d3c2eb897bf1433ee93eef609c6a9'
# These Cargo.lock-verified crate archives provide protoc and its include tree.
PROTOC_CRATES = {
    'x86_64': ('linux-x86_64', 'b38b07546580df720fa464ce124c4b03630a6fb83e05c336fea2a241df7e5d78'),
    'aarch64': ('linux-aarch_64', 'c350df4d49b5b9e3ca79f7e646fde2377b199e13cfa87320308397e1f37e1a4c'),
}


def build(command, cwd, env, evidence):
    process = subprocess.Popen(command, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
    try:
        try:
            out, err = process.communicate(timeout=1800)
            code = process.returncode
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                out, err = process.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                out, err = process.communicate()
            err += b'\noffline all-smi build timed out\n'
            code = 124
        (evidence / 'all-smi-build.stdout').write_bytes(out)
        (evidence / 'all-smi-build.stderr').write_bytes(err)
        return code
    finally:
        if process.poll() is None:
            try:
                os.killpg(process.pid, signal.SIGTERM)
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
            except ProcessLookupError:
                process.wait()


def reproduce(checkout, binary, expected, protoc_crate, evidence, bootstrap_output=None):
    checkout, protoc_crate, evidence = Path(checkout).resolve(), Path(protoc_crate).resolve(), Path(evidence)
    if bootstrap_output is None:
        binary = Path(binary).resolve()
        if sha256_file(binary) != expected or not os.access(binary, os.X_OK):
            raise ValueError('pinned all-smi binary missing, non-executable or changed')
    elif binary is not None or expected is not None:
        raise ValueError('bootstrap and verification modes are exclusive')
    platform_pin = PROTOC_CRATES.get(platform.machine())
    if platform_pin is None or sha256_file(protoc_crate) != platform_pin[1]:
        raise ValueError('protoc archive does not match the architecture-specific Cargo.lock pin')
    revision = subprocess.run(['git', '-C', str(checkout), 'rev-parse', '--verify', 'HEAD'],
                              check=True, capture_output=True, text=True, timeout=20).stdout.strip()
    if revision != SOURCE_REVISION:
        raise ValueError('all-smi checkout is not the unchanged spec revision')
    # Archive the committed tree rather than trusting a possibly dirty checkout.
    archive = subprocess.run(['git', '-C', str(checkout), 'archive', '--format=tar', SOURCE_REVISION],
                             check=True, capture_output=True, timeout=60).stdout
    version = subprocess.run(['rustc', '--version'], check=True, capture_output=True, text=True, timeout=20).stdout.strip()
    cargo = subprocess.run(['cargo', '--version'], check=True, capture_output=True, text=True, timeout=20).stdout.strip()
    epoch = subprocess.run(['git', '-C', str(checkout), 'show', '-s', '--format=%ct', SOURCE_REVISION],
                           check=True, capture_output=True, text=True, timeout=20).stdout.strip()
    if not epoch.isdigit(): raise ValueError('invalid source commit timestamp')
    with tempfile.TemporaryDirectory(prefix='cert-all-smi-') as tmp:
        root = Path(tmp); source = root / 'source'; source.mkdir()
        with tarfile.open(fileobj=io.BytesIO(archive), mode='r:') as tar:
            tar.extractall(source, filter='data')
        tools = root / 'tools'; tools.mkdir()
        with tarfile.open(protoc_crate, mode='r:gz') as tar:
            tar.extractall(tools, filter='data')
        protoc_root = tools / f'protoc-bin-vendored-{platform_pin[0]}-3.2.0'
        if not os.access(protoc_root / 'bin/protoc', os.X_OK) or not (protoc_root / 'include').is_dir():
            raise ValueError('pinned protoc archive lacks executable or includes')
        # Do not pass service credentials or arbitrary compiler flags to build scripts.
        env = {key: os.environ[key] for key in ('PATH', 'RUSTUP_HOME', 'CARGO_HOME', 'LANG', 'LC_ALL', 'TMPDIR') if key in os.environ}
        env['HOME'] = str(root / 'home'); Path(env['HOME']).mkdir()
        env.setdefault('CARGO_HOME', str(Path.home() / '.cargo'))
        env['CARGO_NET_OFFLINE'] = 'true'
        env['SOURCE_DATE_EPOCH'] = epoch
        # Normalize build roots in Rust and C/C++ dependencies. Different temp
        # paths must not silently produce different approved binary bytes.
        cargo_home = str(Path(env['CARGO_HOME']).resolve())
        remaps = [(str(root), '/cert-all-smi'), (cargo_home, '/cert-cargo-home')]
        env['CARGO_ENCODED_RUSTFLAGS'] = '\x1f'.join(f'--remap-path-prefix={source}={target}' for source, target in remaps)
        flags = ' '.join(f'-ffile-prefix-map={source}={target} -fdebug-prefix-map={source}={target}' for source, target in remaps)
        env['CFLAGS'] = flags; env['CXXFLAGS'] = flags
        env['PROTOC'] = str(protoc_root / 'bin/protoc')
        env['PROTOC_INCLUDE'] = str(protoc_root / 'include')
        command = ['cargo', 'build', '--locked', '--offline', '--release', '--bin', 'all-smi', '--target-dir', str(root / 'target')]
        if build(command, source, env, evidence) != 0:
            raise ValueError('offline pinned all-smi build failed')
        built = root / 'target/release/all-smi'
        actual = sha256_file(built)
        (evidence / 'all-smi-built.sha256').write_text(actual + '\n', encoding='utf-8')
        if bootstrap_output is not None:
            output = Path(bootstrap_output)
            with built.open('rb') as source, output.open('xb') as target:
                shutil.copyfileobj(source, target)
            output.chmod(0o755)
        elif actual != expected:
            raise ValueError('pinned all-smi binary differs from fresh offline source build')
    return {'source_revision': revision, 'source_archive_sha256': hashlib.sha256(archive).hexdigest(),
            'binary_sha256': actual, 'protoc_crate_sha256': platform_pin[1], 'rustc': version, 'cargo': cargo,
            'source_date_epoch': epoch, 'build_profile': 'offline-locked-release-remapped-v1'}


def main():
    import argparse
    import json
    import manifest_tool
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ('manifest', 'candidate', 'baseline', 'evidence'):
        parser.add_argument('--' + key, type=Path, required=True)
    args = parser.parse_args()
    try:
        _, config = manifest_tool.load_manifest(args.manifest, 'P1', args.candidate, args.baseline,
                                                os.environ['CUDA_VISIBLE_DEVICES'])
        pin = config['all_smi_binary']
        result = reproduce(config['all_smi_checkout'], pin['path'], pin['sha256'], config['all_smi_protoc_crate']['path'], args.evidence)
        (args.evidence / 'all-smi-provenance.json').write_text(json.dumps(result, indent=2) + '\n', encoding='utf-8')
    except (OSError, ValueError, KeyError, subprocess.SubprocessError, tarfile.TarError) as exc:
        parser.exit(1, f'all-smi provenance failed: {exc}\n')
    return 0


if __name__ == '__main__':
    def interrupted(signum, _frame): raise SystemExit(128 + signum)
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    raise SystemExit(main())
