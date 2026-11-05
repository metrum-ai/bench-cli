# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Prepare and independently reproduce a source-pinned all-smi executable.

CPU-only offline build. The emitted hash can be entered into the frozen P1
manifest only if this command exits successfully. It does not certify a GPU.
"""
import argparse
import json
from pathlib import Path
import signal
import sys

import perf_all_smi


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ('checkout', 'protoc-crate', 'output', 'evidence'):
        parser.add_argument('--' + key, type=Path, required=True)
    args = parser.parse_args()
    args.evidence.mkdir()
    first = args.evidence / 'bootstrap'; first.mkdir()
    second = args.evidence / 'verification'; second.mkdir()
    try:
        initial = perf_all_smi.reproduce(args.checkout, None, None, args.protoc_crate, first, bootstrap_output=args.output)
        verified = perf_all_smi.reproduce(args.checkout, args.output, initial['binary_sha256'], args.protoc_crate, second)
        if initial != verified:
            raise ValueError('build provenance differed between independent offline builds')
        (args.evidence / 'verified-provenance.json').write_text(json.dumps(verified, indent=2) + '\n', encoding='utf-8')
        print(verified['binary_sha256'])
        return 0
    except (OSError, ValueError, KeyError, perf_all_smi.subprocess.SubprocessError, perf_all_smi.tarfile.TarError) as exc:
        print(f'all-smi preparation failed; retain evidence: {exc}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    def interrupted(signum, _frame): raise SystemExit(128 + signum)
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    sys.exit(main())
