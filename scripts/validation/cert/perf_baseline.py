# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Pin and verify the approved v1.7.2 Linux release baseline, offline."""
import hashlib
import platform
import tarfile
from pathlib import Path

from manifest_tool import sha256_file

RELEASE = {
    'x86_64': ('fa093e3c878a50c2ebaad590946f88ad0bff2488d5fe90c57a14acf5b844676d',
               '7dba0b42b22f246814b42874f42fd808420a38b23724d7c357488abfa61b165f'),
    'aarch64': ('e76c85ebe96ea63bf33116ed586a23c9a888c3cb5769b52034604e5cadf8cc93',
                '650403310082ccdf215dca468c3a41beed0cbd38be726c62ae1ec918ff8c76db'),
}


def verify(archive, binary):
    arch = platform.machine()
    if arch not in RELEASE:
        raise ValueError('no pinned v1.7.2 release for this CPU architecture')
    archive_hash, binary_hash = RELEASE[arch]
    if sha256_file(archive) != archive_hash or sha256_file(binary) != binary_hash:
        raise ValueError('v1.7.2 release archive or binary hash mismatch')
    member_name = f'metrum-ai-bench-cli-v1.7.2-{arch}-unknown-linux-gnu/bin/metrum-ai-bench-cli-llm'
    with tarfile.open(Path(archive), 'r:gz') as tar:
        members = [member for member in tar.getmembers() if member.name == member_name]
        if len(members) != 1 or not members[0].isfile():
            raise ValueError('release archive lacks unique regular LLM binary')
        stream = tar.extractfile(members[0])
        if stream is None:
            raise ValueError('release binary is unreadable')
        digest = hashlib.sha256()
        with stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b''):
                digest.update(chunk)
        if digest.hexdigest() != binary_hash:
            raise ValueError('baseline binary is not the verified release asset member')
    return {'release': 'v1.7.2', 'architecture': arch, 'asset_sha256': archive_hash, 'binary_sha256': binary_hash}
