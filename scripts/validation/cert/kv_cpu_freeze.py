#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Freeze one CPU KV envelope without irrelevant server, model or GPU pins."""
import argparse
from pathlib import Path
from kv_lanes.kvc import freeze


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ('selection', 'catalog', 'candidate', 'lane-manifest', 'build-receipt', 'tool-manifest', 'out', 'run-out'):
        parser.add_argument('--' + key, type=Path, required=True)
    parser.add_argument('--case', required=True)
    parser.add_argument('--input', type=Path, action='append', default=[])
    args = parser.parse_args()
    freeze(args)


if __name__ == '__main__':
    main()
