#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Check the actual normalized crate: certification probes are never installable."""
import argparse
import tarfile
import tomllib

PUBLIC_BINS = {'metrum-ai-bench-cli', 'metrum-ai-bench-cli-llm', 'metrum-ai-bench-cli-vlm',
               'metrum-ai-bench-cli-asr', 'metrum-ai-bench-cli-imagegen', 'metrum-ai-bench-cli-prompts',
               'metrum-ai-bench-cli-strategic', 'metrum-ai-bench-cli-mock-server'}
PROBE = 'metrum-ai-bench-cli-cert-kv-cpu'


def check_archive(path):
    with tarfile.open(path, 'r:gz') as archive:
        names = archive.getnames()
        roots = {name.split('/')[0] for name in names}
        if len(roots) != 1:
            raise ValueError('crate must have one root')
        root = next(iter(roots))
        stream = archive.extractfile(root + '/Cargo.toml')
        manifest = tomllib.loads(stream.read().decode())
        bins = {target['name'] for target in manifest['bin']}
        if bins != PUBLIC_BINS:
            raise ValueError('published crate changes the public binary set')
        forbidden = ('src/bin/' + PROBE + '.rs', 'tests/kv_cpu_certification.rs',
                     'scripts/validation/cert/fixtures/kv_cpu/')
        for name in names:
            relative = name.removeprefix(root + '/')
            if any(relative == item or relative.startswith(item) for item in forbidden):
                raise ValueError('certification-only source/fixtures entered the crate')
        return sorted(bins)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('archive')
    args = parser.parse_args()
    print('Public crate binary set unchanged: ' + ', '.join(check_archive(args.archive)))
