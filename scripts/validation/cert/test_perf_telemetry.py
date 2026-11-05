# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic telemetry overlap tests; never GPU evidence."""
import json
import unittest

import perf_telemetry_checks as checks


def fixture():
    uuid = 'GPU-fixture'
    rows = [{'kind': 'run', 'run_id': 'r', 't0_wall': '2026-01-01T00:00:00Z'},
            {'kind': 'request', 'run_id': 'r', 'warmup': False, 't_sent_ns': 1_000_000_000, 't_done_ns': 4_000_000_000}]
    for time in (2_000_000_000, 3_000_000_000):
        for metric, value in (('all_smi_gpu_power_consumption_watts', 100), ('all_smi_gpu_utilization', 50)):
            rows.append({'kind': 'telemetry', 'run_id': 'r', 'src': 'all-smi', 't_ns': time,
                         'metric': metric, 'labels': {'gpu_uuid': uuid}, 'value': value})
    rows.append({'kind': 'summary', 'run_id': 'r', 'partial': False, 'dropped_telemetry_rows': 0, 'scrape_error_rows': 0})
    csv = 'timestamp, uuid, power.draw [W]\n2026/01/01 00:00:02.000, GPU-fixture, 100 W\n2026/01/01 00:00:03.000, GPU-fixture, 101 W\n'
    return rows, csv, uuid


def lines(rows): return '\n'.join(json.dumps(row) for row in rows)


class TelemetryTests(unittest.TestCase):
    def test_two_device_samples_overlap(self):
        rows, csv, uuid = fixture()
        result = checks.evaluate(lines(rows), csv, uuid, 0)
        self.assertEqual(result['device_cli_power_samples'], 2)
        self.assertEqual(result['all_smi_utilization_samples'], 2)

    def test_missing_wrong_device_bad_parser_or_drops_fail(self):
        rows, csv, uuid = fixture()
        with self.assertRaises(ValueError): checks.evaluate(lines(rows), csv, 'GPU-other', 0)
        with self.assertRaises(ValueError): checks.evaluate(lines(rows), csv, uuid, 1)
        rows[-1]['dropped_telemetry_rows'] = 1
        with self.assertRaises(ValueError): checks.evaluate(lines(rows), csv, uuid, 0)

    def test_nonoverlapping_device_samples_fail(self):
        rows, csv, uuid = fixture()
        csv = csv.replace('00:00:02.000', '00:00:12.000').replace('00:00:03.000', '00:00:13.000')
        with self.assertRaises(ValueError): checks.evaluate(lines(rows), csv, uuid, 0)


if __name__ == '__main__': unittest.main()
