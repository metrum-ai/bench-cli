#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Generic single-server dispatch and explicitly deferred layout overrides."""
import csv
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
# These require evidence, identity, cleanup or terminal-class layouts that do
# not follow one common r4 server. They must not silently use this profile.
OVERRIDES = {
    'BC-PERF-001': 'P1 performance/exporter',
    'BC-TELEM-001': 'P1 performance/exporter',
    'BC-PERF-002': 'P2 performance',
    'BC-PERF-003': 'G7 full-grid performance',
    'BC-LIVE-002': 'L2 multimodal server',
    'BC-LIVE-003': 'L3 speech server',
    'BC-LIVE-004': 'L4 image generation server',
    'BC-GPU-011': 'G4 additional VLM server',
    'BC-GPU-015': 'G5 additional endpoint server',
    'BC-COMP-002': 'reference container',
    'BC-COMP-001': 'trigger-bound performance barrier',
    'BC-OPS-001': 'trigger-bound cleanup',
    'BC-INTEG-001': 'external signoff',
}


def validate(manifest, case):
    """Require catalog-matched, exact production dispatch for one common server."""
    if case in OVERRIDES:
        raise ValueError(f'case requires {OVERRIDES[case]} override')
    try:
        with (REPO / 'docs/RELEASE_CERTIFICATION_CATALOG.csv').open(newline='', encoding='utf-8') as stream:
            rows = [row for row in csv.DictReader(stream) if row.get('ID') == case]
        if len(rows) != 1 or rows[0]['Tier'] != 'RC-GPU':
            raise ValueError('case is not uniquely selected from the GPU catalog')
        lane = rows[0]['Parallel group']
        if lane not in {'G1', 'G2', 'G3', 'G4', 'G5', 'G6', 'L1', 'R', 'D', 'V'}:
            raise ValueError('unimplemented common-server lane')
        if manifest['lane'] != lane or manifest['harness_repo'] != str(REPO):
            raise ValueError('dispatch does not match the candidate catalog')
        entry = manifest['cases'][case]
        launcher = str(HERE / 'case_entry.sh')
        command = [launcher, '--case', case, '--lane', lane.lower(),
                   '--candidate', manifest['candidate']['path'], '--baseline', manifest['baseline']['path'],
                   '--manifest', manifest['lane_manifest']['path'], '--out', entry['out']]
        if entry['launcher']['path'] != launcher or entry['command'] != command:
            raise ValueError('dispatch does not match production case entry')
        return lane
    except (KeyError, TypeError) as exc:
        raise ValueError('malformed production dispatch') from exc
