# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""CPU-only strategic oracle tests; not GPU evidence."""
import csv
import io
import json
import unittest

import g5_strategic_checks as checks


class StrategicTests(unittest.TestCase):
    def fixture(self):
        rows = [dict(stage='1', warmup='false', success='true', error='', answer_completed='true', valid='', send_offset_s='0', service_latency_s='2'),
                dict(stage='2', warmup='false', success='false', error='stream_truncated', answer_completed='false', valid='', send_offset_s='3', service_latency_s='1')]
        out = io.StringIO(); writer = csv.DictWriter(out, fieldnames=rows[0]); writer.writeheader(); writer.writerows(rows)
        summary = {'points': [{'load': 1.0, 'n': 1, 'errors': 0, 'goodput': .5}, {'load': 2.0, 'n': 1, 'errors': 1, 'goodput': 0.0}]}
        trace = '\n'.join(map(json.dumps, [{'request': 1, 'kind': 'complete', 'monotonic_ns': 1},
                                          {'request': 2, 'kind': 'error', 'monotonic_ns': 3}]))
        kill = {'kill_exit': 0, 'start_exit': 0, 'kill_start_ns': 2, 'kill_end_ns': 4}
        return summary, out.getvalue(), trace, kill

    def test_counts_goodput_and_export_failure(self):
        summary, data, trace, kill = self.fixture()
        encode = lambda: json.dumps(summary)
        export_summary = json.dumps({'points': [{'n': 1, 'errors': 0, 'answered_requests': 1}]})
        self.assertEqual(1, checks.verify(encode(), data, trace, kill, 1, export_summary, 1)['errors'])
        with self.assertRaises(ValueError):
            checks.verify(encode(), data, trace, kill, 1, export_summary, 0)
        summary['points'][0]['goodput'] = 1.0
        with self.assertRaises(ValueError):
            checks.verify(encode(), data, trace, kill, 1, export_summary, 1)
        with self.assertRaises(ValueError):
            checks.verify(encode(), data, trace.replace('"error"', '"complete"'), kill, 1, export_summary, 1)


if __name__ == '__main__':
    unittest.main()
