# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Negative controls for r5 wire/counter intake; never GPU evidence."""
import base64
import json
import tempfile
import unittest
from pathlib import Path

import kv_r5_metrics as metrics
import kv_r5_proxy as proxy
import kv_r5_runtime as runtime


class MetricControls(unittest.TestCase):
    def setUp(self):
        self.specs = {'hits_total': ('counter', 'tokens'), 'rate': ('gauge', 'ratio')}
        self.page = '# TYPE hits counter\nhits_total{worker="0"} 4\n# TYPE rate gauge\nrate 0.5\n'

    def test_counter_and_gauge_keep_units(self):
        rows = metrics.deltas(self.page, self.page.replace('} 4', '} 8'), self.specs)
        self.assertEqual(metrics.single(rows, 'hits_total'), 4)
        self.assertIsNone(rows[1]['delta'])
        self.assertEqual(rows[1]['unit'], 'ratio')

    def test_missing_reset_label_change_and_precision_are_rejected(self):
        for after in [self.page.replace('} 4', '} 3'), self.page.replace('worker="0"', 'worker="1"'),
                      self.page.replace('} 4', '} 9007199254740994'),
                      self.page.replace('hits_total{worker="0"} 4\n', ''),
                      self.page.replace('rate 0.5', 'rate NaN'), self.page.replace('rate 0.5', 'rate 1.1'),
                      self.page.replace('hits_total{worker="0"} 4', 'hits_total{worker="0"} 4.5'),
                      self.page+'hits_total{worker="0"} 4\n',
                      self.page+'# TYPE hits gauge\n', self.page.replace('# TYPE hits counter\n', '')]:
            with self.subTest(after=after), self.assertRaises(ValueError):
                metrics.deltas(self.page, after, self.specs)

    def test_replica_counters_cannot_be_summed(self):
        page = self.page+'hits_total{worker="1"} 2\n'
        with self.assertRaises(ValueError):
            metrics.single(metrics.deltas(page, page, self.specs), 'hits_total')


class WireControls(unittest.TestCase):
    def test_entire_conversation_is_hashed(self):
        a = {'messages': [{'role': 'user', 'content': 'A'}, {'role': 'assistant', 'content': 'B'}], 'stream': True}
        b = {**a, 'messages': list(reversed(a['messages']))}
        self.assertNotEqual(proxy.identity(json.dumps(a).encode()), proxy.identity(json.dumps(b).encode()))

    def test_missing_usage_and_reordered_requests_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            expected = {'messages_sha256': 'a'*64, 'options': {'stream': True}}
            events = [{'request': 1, 'kind': 'request_identity', **expected},
                      {'request': 1, 'kind': 'chunk', 'bytes_b64': base64.b64encode(b'data: [DONE]\n\n').decode()},
                      {'request': 1, 'kind': 'complete'}]
            (directory/'trace.jsonl').write_text('\n'.join(map(json.dumps, events)))
            phase = {'directory': directory, 'plan': {'expected_requests': [expected]}}
            with self.assertRaises(ValueError):
                runtime.wire(phase)
            phase['plan']['expected_requests'][0] = {**expected, 'messages_sha256': 'b'*64}
            with self.assertRaisesRegex(ValueError, 'wire order'):
                runtime.wire(phase)


if __name__ == '__main__':
    unittest.main()
