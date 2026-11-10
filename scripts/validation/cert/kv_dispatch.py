#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Engine-specific KV dispatch registry; unavailable runners fail closed."""
import argparse
import importlib

CASE_LANES = {
    'BC-KV-001': 'KVV', 'BC-KV-002': 'KVV', 'BC-KV-003': 'KVS',
    'BC-KV-004': 'KVS', 'BC-KV-005': 'KVL', 'BC-KV-006': 'KVD',
    'BC-KV-007': 'KVT', 'BC-KV-008': 'KVV', 'BC-KV-009': 'KVV',
    'BC-KV-010': 'KVC', 'BC-KV-011': 'KVC', 'BC-KV-012': 'KVC',
    'BC-KV-013': 'KVC', 'BC-KV-014': 'KVC', 'BC-KV-015': 'KVC',
}
# Each engine lane replaces only its own entry with its reviewed module.
# None of these can fall back to the r4 common-server adapter.
LANE_MODULES = {
    'KVV': 'kv_lanes.kvv',
    'KVS': 'kv_lanes.kvs',
    'KVL': 'kv_lanes.kvl',
    'KVD': 'kv_lanes.kvd',
    'KVT': 'kv_lanes.kvt',
    'KVC': 'kv_lanes.kvc',
}


def require_lane(case, lane):
    if CASE_LANES.get(case) != lane:
        raise ValueError('KV case/lane does not match the registered catalog mapping')


def adapter(case, lane):
    require_lane(case, lane)
    module = importlib.import_module(LANE_MODULES[lane])
    if not callable(getattr(module, 'main', None)) or not callable(getattr(module, 'freeze', None)):
        raise ValueError('KV adapter must implement main(argv) and freeze(args)')
    return module


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--case', required=True)
    parser.add_argument('--lane', required=True, choices=sorted(LANE_MODULES))
    args, remaining = parser.parse_known_args()
    try:
        return adapter(args.case, args.lane).main(['--case', args.case, *remaining])
    except ValueError as exc:
        parser.exit(1, f'{exc}\n')


if __name__ == '__main__':
    raise SystemExit(main())
