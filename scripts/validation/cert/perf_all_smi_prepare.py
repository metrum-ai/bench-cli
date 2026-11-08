# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Stage a downloaded fork release offline; never overwrite an existing output."""
import argparse
import json
from pathlib import Path
import tarfile

import perf_all_smi
from manifest_tool import sha256_file


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ('asset', 'checksum', 'output', 'evidence'):
        parser.add_argument('--' + key, type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        checksum_sha = sha256_file(args.checksum)
        data, _ = perf_all_smi.inspect_release_asset(args.asset, args.checksum, checksum_sha)
        with args.output.open('xb') as output:
            output.write(data)
        args.output.chmod(0o755)
        binary_sha = sha256_file(args.output)
        perf_all_smi.verify_release_asset(args.output, binary_sha, args.asset,
                                          args.checksum, checksum_sha, args.evidence)
        pins = {key: {'path': str(path.resolve()), 'sha256': sha256_file(path)} for key, path in (
            ('all_smi_binary', args.output), ('all_smi_release_asset', args.asset),
            ('all_smi_published_checksum', args.checksum))}
        print(json.dumps(pins, indent=2))
    except (OSError, ValueError, tarfile.TarError) as exc:
        parser.exit(1, f'all-smi preparation failed: {exc}\n')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
