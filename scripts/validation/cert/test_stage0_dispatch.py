#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline generic dispatch and explicit layout-override tests."""
import csv
import unittest

import stage0_dispatch as dispatch


class DispatchTests(unittest.TestCase):
    def manifest(self, case, lane):
        entry = str(dispatch.HERE / 'case_entry.sh')
        return {'lane': lane, 'harness_repo': str(dispatch.REPO), 'candidate': {'path': '/candidate'},
                'baseline': {'path': '/baseline'}, 'lane_manifest': {'path': '/lane'},
                'cases': {case: {'out': '/out', 'launcher': {'path': entry},
                    'command': [entry, '--case', case, '--lane', lane.lower(), '--candidate', '/candidate',
                                '--baseline', '/baseline', '--manifest', '/lane', '--out', '/out']}}}

    def test_common_catalog_lanes(self):
        for case, lane in [('BC-GPU-001', 'G1'), ('BC-GPU-004', 'G2'), ('BC-GPU-008', 'G3'),
                           ('BC-GPU-010', 'G4'), ('BC-GPU-013', 'G5'), ('BC-COMP-003', 'G6'), ('BC-PERF-003', 'G7'),
                           ('BC-LIVE-001', 'L1'), ('BC-LIVE-002', 'L2'), ('BC-REASON-001', 'R'), ('BC-REASON-006', 'D'),
                           ('BC-REASON-005', 'V')]:
            with self.subTest(case=case): self.assertEqual(dispatch.validate(self.manifest(case, lane), case), lane)

    def test_every_non_override_gpu_catalog_case_is_routed(self):
        catalog = dispatch.REPO / 'docs/RELEASE_CERTIFICATION_CATALOG.csv'
        with catalog.open(newline='', encoding='utf-8') as stream:
            rows = [row for row in csv.DictReader(stream) if row.get('Tier') == 'RC-GPU']
        cases = [row for row in rows if row['ID'] not in dispatch.OVERRIDES]
        self.assertGreater(len(cases), 1)
        for row in cases:
            with self.subTest(case=row['ID']):
                self.assertEqual(dispatch.validate(self.manifest(row['ID'], row['Parallel group']), row['ID']), row['Parallel group'])

    def test_layout_overrides_never_use_common_adapter(self):
        for case in dispatch.OVERRIDES:
            with self.subTest(case=case), self.assertRaisesRegex(ValueError, 'override'):
                dispatch.validate({}, case)

    def test_wrong_catalog_lane_and_modified_command_rejected(self):
        with self.assertRaisesRegex(ValueError, 'catalog'):
            dispatch.validate(self.manifest('BC-GPU-001', 'G2'), 'BC-GPU-001')
        value = self.manifest('BC-GPU-001', 'G1')
        value['cases']['BC-GPU-001']['command'].append('--extra')
        with self.assertRaisesRegex(ValueError, 'production case entry'):
            dispatch.validate(value, 'BC-GPU-001')
        with self.assertRaisesRegex(ValueError, 'GPU catalog'):
            dispatch.validate(self.manifest('BC-GPU-999', 'G1'), 'BC-GPU-999')


if __name__ == '__main__': unittest.main()
