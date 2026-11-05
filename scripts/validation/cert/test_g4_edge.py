# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""CPU-only synthetic edge-case oracles; not GPU evidence."""
import base64
import json
import unittest

import g4_edge_checks as checks


class EdgeTests(unittest.TestCase):
    def test_zero_window_requires_no_requests_and_null_rates(self):
        summary = {'schema_version': 'bench.summary.v5', 'attempted': 0, 'window_seconds': 0,
                   'goodput': {'requests_per_second': None}}
        self.assertEqual(0, checks.empty(json.dumps(summary), '', 0)['attempted'])
        summary['requests_per_second'] = 0
        with self.assertRaises(ValueError):
            checks.empty(json.dumps(summary), '', 0)
        with self.assertRaises(ValueError):
            checks.empty(json.dumps(summary), '{"request":1}', 0)

    def fixture(self, fallback):
        rows = []
        events = []
        original = b'data: {"choices":[{"delta":{"content":"a"}}]}\n\ndata: {"choices":[],"usage":{"prompt_tokens":2,"completion_tokens":3}}\n\ndata: [DONE]\n\n'
        forwarded = b'data: {"choices":[{"delta":{"content":"a"}}]}\n\ndata: [DONE]\n\n'
        for seq in range(2):
            row = {'schema_version': 'bench.request.v5', 'phase': 'measure', 'seq': seq, 'send_offset_s': seq,
                   'latency_s': 1, 'error': None, 'answer_completed': True,
                   'usage_prompt_tokens_present': False, 'usage_completion_tokens_present': False}
            if fallback: row['tokenized_completion_tokens'] = 3
            rows.append(row)
            events += [{'request': seq, 'kind': 'chunk', 'bytes_b64': base64.b64encode(original).decode()},
                       {'request': seq, 'kind': 'transformation', 'mode': 'remove_usage'},
                       {'request': seq, 'kind': 'forwarded_chunk', 'bytes_b64': base64.b64encode(forwarded).decode()},
                       {'request': seq, 'kind': 'complete'}]
        rows.append({'schema_version': 'bench.summary.v5', 'attempted': 2,
                     'legacy_success_completion_tokens_per_second': 3 if fallback else None})
        return rows, events

    def test_missing_usage_and_fallback_fail_closed(self):
        for fallback in (False, True):
            rows, events = self.fixture(fallback)
            data = lambda: '\n'.join(map(json.dumps, rows))
            trace = '\n'.join(map(json.dumps, events))
            self.assertEqual(2, checks.missing(data(), trace, 0, 2, fallback)['requests'])
            rows[-1]['completion_tokens_per_second'] = 1
            with self.assertRaises(ValueError):
                checks.missing(data(), trace, 0, 2, fallback)
            rows[-1]['completion_tokens_per_second'] = None
            if fallback:
                rows[0]['tokenized_completion_tokens'] = None
                with self.assertRaises(ValueError):
                    checks.missing(data(), trace, 0, 2, fallback)


if __name__ == '__main__':
    unittest.main()
