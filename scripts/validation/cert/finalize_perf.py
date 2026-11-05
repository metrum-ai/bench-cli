# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Publish provisional perf results only after the shell has checked cleanup."""
import argparse
import datetime as dt
import json
from pathlib import Path

import result_writer


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--fail', action='store_true')
    parser.add_argument('cases', nargs='+')
    args = parser.parse_args()
    cleanup = args.out / 'evidence/cleanup-verified.json'
    if not args.fail and not cleanup.is_file():
        raise ValueError('verified cleanup marker missing')
    for case in args.cases:
        provisional = args.out / 'evidence' / f'provisional-{case}.json'
        if not provisional.is_file():
            continue  # common failure writer supplies a missing result
        record = json.loads(provisional.read_text(encoding='utf-8'))
        if record.get('ID') != case:
            raise ValueError('provisional case identity mismatch')
        record['end_utc'] = dt.datetime.now(dt.timezone.utc).isoformat().replace('+00:00', 'Z')
        if args.fail:
            record['result'] = 'FAIL'
            record['observed_values']['finalization_error'] = 'lane failed or cleanup was not verified'
        else:
            record['evidence_paths'].append(str(cleanup.relative_to(args.out)))
        record['evidence_paths'].append(str(provisional.relative_to(args.out)))
        for name in ('server.log', 'server-stop.txt', 'perf-emergency-server.log', 'perf-emergency-stop.txt'):
            path = args.out / 'evidence' / name
            relative = str(path.relative_to(args.out))
            if path.is_file() and relative not in record['evidence_paths']:
                record['evidence_paths'].append(relative)
        result_writer.write_record(args.out, record)
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
