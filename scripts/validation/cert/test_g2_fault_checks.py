# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic fault oracles only, not GPU evidence."""
import base64
import json
import unittest

import g2_fault_checks


class FaultChecksTests(unittest.TestCase):
    def fixture(self, fault, target):
        kind = 'timeout' if fault == 'timeout' else 'parse_error'
        rows = []
        events = []
        for request in (1, 2):
            failed = request == target
            rows.append({'schema_version': 'bench.request.v5', 'phase': 'measure', 'seq': request-1,
                         'answer_completed': False if failed else True,
                         'error': {'kind': kind} if failed else None, 'latency_s': 2.01 if failed else .2})
            original = b'data: {"choices":[]}\n\n'
            forwarded = b'data: {"cho\n\n' if failed and fault == 'corrupt' else original
            events += [{'request': request, 'kind': 'send'},
                       {'request': request, 'kind': 'chunk', 'bytes_b64': base64.b64encode(original).decode()}]
            if failed:
                events.append({'request': request, 'kind': 'delay', 'seconds': 4.5} if fault == 'timeout' else
                              {'request': request, 'kind': 'transformation', 'mode': 'corrupt'})
            events.append({'request': request, 'kind': 'forwarded_chunk', 'bytes_b64': base64.b64encode(forwarded).decode()})
        rows.append({'schema_version': 'bench.summary.v5', 'attempted': 2, 'errors': 1, 'successes': 1, 'errors_by_type': {kind: 1}})
        return '\n'.join(map(json.dumps, rows)), '\n'.join(map(json.dumps, events))

    def test_fault_orders_and_fail_closed(self):
        for fault in ('timeout', 'corrupt'):
            for target in (1, 2):
                data, trace = self.fixture(fault, target)
                observed = g2_fault_checks.verify(data, trace, fault, target, 1)
                self.assertEqual(target - 1, observed['failure_slot'])
                with self.assertRaises(ValueError):
                    g2_fault_checks.verify(data, trace, fault, target, 0)
                with self.assertRaises(ValueError):
                    g2_fault_checks.verify(data, trace, fault, 3-target, 1)
                if fault == 'timeout':
                    with self.assertRaises(ValueError):
                        g2_fault_checks.verify(data.replace('2.01', '8.0'), trace, fault, target, 1)


if __name__ == '__main__':
    unittest.main()
