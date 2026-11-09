#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Prepare a pinned product SUT declaration from observed inventory and a lane.

This creates a distinct input before freezing; it does not alter the separate
certification SUT, runtime records, or validator inputs after the freeze.
"""
import argparse
import json
import math
import re
from pathlib import Path

import manifest_tool


def pinned_json(path, sha):
    path = Path(path).resolve(strict=True)
    if (not path.is_file() or not re.fullmatch(r'[a-f0-9]{64}', sha)
            or manifest_tool.sha256_file(path) != sha):
        raise ValueError('inventory or lane pin mismatch')
    data = json.loads(path.read_text(encoding='utf-8'), object_pairs_hook=manifest_tool.unique_object)
    if not isinstance(data, dict):
        raise ValueError('inventory and lane must be JSON objects')
    return data


def required_text(value, label):
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f'{label} must be nonempty')
    return value


def memory(value, label):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value < 1:
        raise ValueError(f'{label} must be finite and at least one GiB')
    return math.floor(value)


def declaration(actual, config):
    gpus = actual.get('gpus')
    if not isinstance(gpus, list) or len(gpus) != 1 or not isinstance(gpus[0], dict):
        raise ValueError('exactly one observed GPU is required')
    gpu = gpus[0]
    name = required_text(gpu.get('name'), 'GPU name')
    driver = required_text(gpu.get('driver'), 'GPU driver')
    host_os = required_text(actual.get('host_os'), 'host OS')
    model = required_text(config.get('model'), 'model')
    revision = required_text(config.get('revision'), 'revision')
    image = required_text(config.get('image'), 'image')
    if not manifest_tool.MODEL.fullmatch(model) or not manifest_tool.REV.fullmatch(revision) or not manifest_tool.IMAGE.fullmatch(image):
        raise ValueError('server model, revision and image must be pinned')
    flags = config.get('server_flags', [])
    if not isinstance(flags, list) or any(not isinstance(x, str) or not x or '\0' in x for x in flags):
        raise ValueError('server flags must be an argument array')
    if any(flag.split('=', 1)[0] in manifest_tool.FORBIDDEN for flag in flags):
        raise ValueError('server flags must not override pinned identity')
    return {'provenance': 'mixed', 'name': f'certification actual {name} x1',
            'gpu': {'model': name, 'count': 1, 'memory_gb': memory(gpu.get('memory_gb'), 'GPU memory')},
            'memory_gb': memory(actual.get('memory_gb'), 'system memory'), 'driver_version': driver,
            'runtime': {'name': 'vllm', 'version': image, 'config': json.dumps(flags, separators=(',', ':'))},
            'model': {'id': model, 'revision': revision}, 'host_os': host_os,
            'field_provenance': {'gpu': 'observed', 'memory_gb': 'observed', 'driver_version': 'observed',
                                 'host_os': 'observed', 'runtime': 'declared', 'model': 'declared'}}


def prepare(inventory, inventory_sha, lane_manifest, lane_sha, lane, out):
    actual = pinned_json(inventory, inventory_sha)
    manifest = pinned_json(lane_manifest, lane_sha)
    config = manifest.get('lanes', {}).get(lane)
    if not isinstance(config, dict):
        raise ValueError('lane configuration is missing')
    sut = declaration(actual, config)
    out = Path(out)
    with out.open('x', encoding='utf-8') as stream:
        json.dump(sut, stream, sort_keys=True, indent=2, allow_nan=False)
        stream.write('\n')
    return {'path': str(out.resolve()), 'sha256': manifest_tool.sha256_file(out)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('inventory', 'inventory-sha256', 'lane-manifest', 'lane-sha256', 'lane', 'out'):
        parser.add_argument('--' + name, required=True)
    args = parser.parse_args()
    try:
        pin = prepare(args.inventory, args.inventory_sha256, args.lane_manifest, args.lane_sha256, args.lane, args.out)
    except (ValueError, OSError, TypeError) as exc:
        parser.exit(1, f'product SUT preparation failed: {exc}\n')
    print(json.dumps(pin, sort_keys=True))


if __name__ == '__main__':
    main()
