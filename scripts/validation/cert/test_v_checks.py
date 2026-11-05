# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic CPU-only VLM assertion tests, never GPU evidence."""
import base64
import json
import unittest

import v_checks


def fixture():
    digest = 'a' * 64
    rows, events = [], []
    for i in range(1, 8):
        start = i * 1_000_000_000
        rows.append({'schema_version': 'metrum.request.v3', 'phase': 'measure', 'seq': i,
                     'latency_s': .004, 'ttft_s': .002, 'ttft_visible_answer_s': .003,
                     'ttft_source': 'stream', 'answer_completed': True, 'error': None,
                     'modality_metrics': {'image_count': 1}})
        events.extend([{'request': i, 'kind': 'image_identity', 'monotonic_ns': start-1, 'image_sha256': [digest],
                        'model': 'model', 'thinking': True},
                       {'request': i, 'kind': 'send', 'monotonic_ns': start}])
        for offset, delta in [(2_000_000, {'reasoning': 'r'}), (3_000_000, {'content': 'a'})]:
            raw = ('data: ' + json.dumps({'choices': [{'index': 0, 'delta': delta}]}) + '\n\n').encode()
            events.append({'request': i, 'kind': 'chunk', 'monotonic_ns': start+offset,
                           'bytes_b64': base64.b64encode(raw).decode()})
        events.append({'request': i, 'kind': 'complete', 'monotonic_ns': start+4_000_000})
    return rows, events, digest


def lines(items): return '\n'.join(json.dumps(item) for item in items)


class VChecksTests(unittest.TestCase):
    def test_seven_pinned_real_image_shape_and_clocks(self):
        rows, events, digest = fixture()
        self.assertEqual(v_checks.evaluate(lines(rows), lines(events), 0, digest, 'model')['completed'], 7)

    def test_missing_image_or_thinking_fails(self):
        rows, events, digest = fixture()
        rows[0]['modality_metrics']['image_count'] = 0
        with self.assertRaises(ValueError): v_checks.evaluate(lines(rows), lines(events), 0, digest, 'model')
        rows, events, digest = fixture(); events[0]['thinking'] = False
        with self.assertRaises(ValueError): v_checks.evaluate(lines(rows), lines(events), 0, digest, 'model')

    def test_no_reasoning_fails(self):
        rows, events, digest = fixture()
        for row in rows: row['ttft_visible_answer_s'] = .002
        for event in events:
            if event['kind'] == 'chunk':
                raw = base64.b64decode(event['bytes_b64']).replace(b'"reasoning": "r"', b'"content": "r"')
                event['bytes_b64'] = base64.b64encode(raw).decode()
        with self.assertRaises(ValueError): v_checks.evaluate(lines(rows), lines(events), 0, digest, 'model')

    def test_six_or_client_failure_fails(self):
        rows, events, digest = fixture()
        with self.assertRaises(ValueError): v_checks.evaluate(lines(rows[:-1]), lines(events), 0, digest, 'model')
        with self.assertRaises(ValueError): v_checks.evaluate(lines(rows), lines(events), 1, digest, 'model')


if __name__ == '__main__': unittest.main()
