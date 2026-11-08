# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Verify offline fork release assets and their pinned published checksums."""
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import tarfile

from manifest_tool import sha256_file

FORK_REPO = 'metrum-ai/all-smi'
RELEASE_TAG = 'v0.28.1-metrum.3'
ASSETS = {
    'all-smi-linux-x86_64.tar.gz': ('x86_64', 'gnu', '99b33cf31b14e6bc44b45b44a50b996aab99b5181d5a23c7d48886d084c29f4e'),
    'all-smi-linux-x86_64-musl.tar.gz': ('x86_64', 'musl', '717efefb9a2df3e326500e2f33858cb21e05300fad858586f75985870c364d5a'),
    'all-smi-linux-aarch64.tar.gz': ('aarch64', 'gnu', '7c0e869995aee2731323ecfb77989c59a18e3a5a3553289375e7a8609bbd2e82'),
}
BINARY_NAME = 'all-smi'


def inspect_release_asset(asset, checksum, checksum_sha256):
    """Return verified binary bytes and provenance, without executing/extracting paths."""
    asset = Path(asset)
    checksum = Path(checksum)
    if asset.name not in ASSETS:
        raise ValueError('unsupported all-smi release asset')
    arch, libc, published = ASSETS[asset.name]
    if not checksum.is_file():
        raise ValueError('published checksum file missing')
    checksum_bytes = checksum.read_bytes()
    checksum_hash = hashlib.sha256(checksum_bytes).hexdigest()
    if checksum_hash != checksum_sha256:
        raise ValueError('published checksum file pin mismatch')
    try:
        lines = [line for line in checksum_bytes.decode('utf-8').splitlines() if line.strip()]
    except UnicodeError as exc:
        raise ValueError('malformed published checksum') from exc
    if len(lines) != 1 or not re.fullmatch(r'[0-9a-fA-F]{64}  ' + re.escape(asset.name), lines[0]):
        raise ValueError('malformed or wrong-name published checksum')
    if lines[0][:64].lower() != published:
        raise ValueError('published checksum differs from table pin')
    if not asset.is_file() or sha256_file(asset) != published:
        raise ValueError('release asset does not match published checksum')
    with tarfile.open(asset, 'r:gz') as tar:
        members = [m for m in tar.getmembers() if Path(m.name).name == BINARY_NAME and m.isfile()]
        if len(members) != 1:
            raise ValueError('release asset must contain exactly one regular all-smi binary')
        data = tar.extractfile(members[0]).read()
    if len(data) < 20 or data[:4] != b'\x7fELF' or data[5] not in (1, 2):
        raise ValueError('all-smi binary must have a valid ELF header')
    machine = int.from_bytes(data[18:20], 'little' if data[5] == 1 else 'big')
    host = platform.machine().lower()
    host = {'amd64': 'x86_64', 'arm64': 'aarch64'}.get(host, host)
    if machine != {'x86_64': 62, 'aarch64': 183}[arch] or host != arch:
        raise ValueError('all-smi ELF architecture or host mismatch')
    url = f'https://github.com/{FORK_REPO}/releases/download/{RELEASE_TAG}/{asset.name}'
    return data, {
        'provenance': 'official-fork-release-asset',
        'repo': FORK_REPO, 'release_tag': RELEASE_TAG,
        'asset_name': asset.name, 'arch': arch, 'libc': libc,
        'asset_url': url, 'published_checksum_url': url + '.sha256',
        'published_asset_sha256': published,
        'published_checksum_file_sha256': checksum_hash,
        'asset_sha256': published,
        'binary_sha256': hashlib.sha256(data).hexdigest(),
        'rebuild_equality_required': False,
    }


def verify_release_asset(binary, binary_sha256, asset, checksum, checksum_sha256, evidence):
    """Verify staged executable equality and write release provenance."""
    data, result = inspect_release_asset(asset, checksum, checksum_sha256)
    binary = Path(binary)
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise ValueError('pinned all-smi binary missing or non-executable')
    if sha256_file(binary) != binary_sha256:
        raise ValueError('pinned all-smi binary changed')
    if result['binary_sha256'] != binary_sha256:
        raise ValueError('pinned all-smi binary is not the binary carried by the pinned release asset')
    evidence = Path(evidence)
    evidence.mkdir(parents=True, exist_ok=True)
    (evidence / 'all-smi-provenance.json').write_text(json.dumps(result, indent=2) + '\n', encoding='utf-8')
    return result


def main():
    import argparse
    import manifest_tool
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ('manifest', 'candidate', 'baseline', 'evidence'):
        parser.add_argument('--' + key, type=Path, required=True)
    args = parser.parse_args()
    try:
        _, config = manifest_tool.load_manifest(args.manifest, 'P1', args.candidate, args.baseline,
                                                os.environ['CUDA_VISIBLE_DEVICES'])
        pin = config['all_smi_binary']
        asset = config['all_smi_release_asset']
        checksum = config['all_smi_published_checksum']
        verify_release_asset(pin['path'], pin['sha256'], asset['path'], checksum['path'], checksum['sha256'], args.evidence)
    except (OSError, ValueError, KeyError, tarfile.TarError) as exc:
        parser.exit(1, f'all-smi provenance failed: {exc}\n')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
