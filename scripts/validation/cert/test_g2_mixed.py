# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline synthetic fixtures; never GPU evidence."""
import base64
import json
import unittest

import g2_mixed_checks


class MixedChecksTests(unittest.TestCase):
    def fixture(self, order, strict=False):
        events, rows = [], []
        failed = 1 if order == 'error-first' else 15
        for request in range(1, 17):
            now = request * 1_000_000_000
            events.extend([{'request': request, 'kind': 'request_transform', 'forwarded_cap': 128 if request % 2 else 4096},
                           {'request': request, 'kind': 'send', 'monotonic_ns': now}])
            if request == failed:
                row = {'outcome': None, 'answer_completed': None, 'error': {'kind': 'stream_truncated'}}
                events.append({'request': request, 'kind': 'error', 'monotonic_ns': now + 100})
            else:
                truncated = request % 2 == 1
                choice = {'index': 0, 'delta': {'reasoning_content': 'r'} if truncated else {'content': 'a'},
                          'finish_reason': 'length' if truncated else 'stop'}
                raw = b'data: ' + json.dumps({'choices': [choice]}).encode() + b'\n\n'
                events.extend([{'request': request, 'kind': 'chunk', 'bytes_b64': base64.b64encode(raw).decode()},
                               {'request': request, 'kind': 'complete', 'monotonic_ns': now + 100}])
                row = {'outcome': 'reasoning_truncated' if truncated else 'answer', 'answer_completed': not truncated,
                       'error': {'kind': 'reasoning_truncated'} if truncated and strict else None}
            rows.append({'schema_version': 'bench.request.v5', 'phase': 'measure', 'seq': request - 1, **row})
        errors = sum(r['error'] is not None for r in rows)
        rows.append({'schema_version': 'bench.summary.v5', 'attempted': 16, 'successes': 16-errors, 'errors': errors, 'answered_requests': 8})
        return '\n'.join(map(json.dumps, rows)), '\n'.join(map(json.dumps, events))

    def test_serial_order_strict_and_counts(self):
        for order in ('error-first', 'error-last'):
            for strict in (False, True):
                data, trace = self.fixture(order, strict)
                result = g2_mixed_checks.verify(data, trace, order, strict)
                self.assertEqual(8, result['answer'])
                self.assertEqual(7, result['reasoning_truncated'])
                self.assertEqual(1, result['failure'])
                with self.assertRaises(ValueError):
                    g2_mixed_checks.verify(data, trace, 'error-last' if order == 'error-first' else 'error-first', strict)
                with self.assertRaises(ValueError):
                    g2_mixed_checks.verify(data, trace, order, not strict)
                with self.assertRaises(ValueError):
                    g2_mixed_checks.verify(data.replace('"answered_requests": 8', '"answered_requests": 7'), trace, order, strict)


if __name__ == '__main__':
    unittest.main()
