# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Verify the pinned all-smi release asset against its published checksum.

all-smi provenance is the official metrum-ai/all-smi fork release asset whose
sha256 matches the release's published checksum, recorded with the source URL,
tag and hash. Reproducing the binary byte-for-byte from source is NOT a
requirement; the previous offline rebuild-equality check was a wrong
requirement and is removed. Everything is offline: the release tarball and its
published checksum are pinned inputs, and the pinned binary must be the binary
carried by that exact tarball.
"""
import hashlib
import io
import json
import os
from pathlib import Path
import tarfile

from manifest_tool import sha256_file

FORK_REPO = 'metrum-ai/all-smi'
RELEASE_TAG = 'v0.28.1-metrum.3'
ASSET_NAME = 'all-smi-linux-x86_64.tar.gz'
ASSET_URL = f'https://github.com/{FORK_REPO}/releases/download/{RELEASE_TAG}/{ASSET_NAME}'
PUBLISHED_CHECKSUM_URL = ASSET_URL + '.sha256'
# Published by the release (all-smi-linux-x86_64.tar.gz.sha256).
PUBLISHED_ASSET_SHA256 = '99b33cf31b14e6bc44b45b44a50b996aab99b5181d5a23c7d48886d084c29f4e'
BINARY_NAME = 'all-smi'


def _extract_binary(asset_path):
    """Return (name, sha256) of the all-smi binary inside the release tarball."""
    with tarfile.open(asset_path, mode='r:gz') as tar:
        member = next((m for m in tar.getmembers() if Path(m.name).name == BINARY_NAME and m.isfile()), None)
        if member is None:
            raise ValueError('release asset does not contain the all-smi binary')
        data = tar.extractfile(member).read()
    return BINARY_NAME, hashlib.sha256(data).hexdigest()


def verify_release_asset(binary, binary_sha256, asset, evidence):
    """Verify the pinned binary is the pinned fork release asset's binary.

    binary:  path to the staged all-smi executable.
    binary_sha256: pinned sha256 of that executable.
    asset:   path to the pinned release tarball (all-smi-linux-x86_64.tar.gz).
    evidence: directory receiving all-smi-provenance.json.
    """
    binary = Path(binary).resolve()
    asset = Path(asset).resolve()
    evidence = Path(evidence).resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise ValueError('pinned all-smi binary missing or non-executable')
    if sha256_file(binary) != binary_sha256:
        raise ValueError('pinned all-smi binary changed')
    if not asset.is_file():
        raise ValueError('pinned all-smi release asset missing')
    asset_sha256 = sha256_file(asset)
    if asset_sha256 != PUBLISHED_ASSET_SHA256:
        raise ValueError('all-smi release asset does not match the published checksum')
    _, extracted_sha256 = _extract_binary(asset)
    if extracted_sha256 != binary_sha256:
        raise ValueError('pinned all-smi binary is not the binary carried by the pinned release asset')
    result = {
        'provenance': 'official-fork-release-asset',
        'repo': FORK_REPO,
        'release_tag': RELEASE_TAG,
        'asset_name': ASSET_NAME,
        'asset_url': ASSET_URL,
        'published_checksum_url': PUBLISHED_CHECKSUM_URL,
        'published_asset_sha256': PUBLISHED_ASSET_SHA256,
        'asset_sha256': asset_sha256,
        'binary_name': BINARY_NAME,
        'binary_sha256': binary_sha256,
        'rebuild_equality_required': False,
    }
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
        verify_release_asset(pin['path'], pin['sha256'], asset['path'], args.evidence)
    except (OSError, ValueError, KeyError, tarfile.TarError) as exc:
        parser.exit(1, f'all-smi provenance failed: {exc}\n')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
