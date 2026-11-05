# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline synthetic provenance tests; never treated as GPU evidence."""
import base64
import json
import unittest

import g1_variants_checks as checks
import sse_reorder
from test_sse_reorder import frame


def captured(source, forwarded, mode=None):
    events = [{'request': 1, 'kind': 'send', 'monotonic_ns': 1},
              {'request': 1, 'kind': 'headers', 'monotonic_ns': 2, 'status': 200}]
    for i, value in enumerate(source):
        events.append({'request': 1, 'kind': 'chunk', 'monotonic_ns': 10+i,
                       'bytes_b64': base64.b64encode(value).decode()})
    if mode:
        events.append({'request': 1, 'kind': 'transformation', 'monotonic_ns': 99, 'mode': mode})
    for i, value in enumerate(forwarded):
        events.append({'request': 1, 'kind': 'forwarded_chunk', 'monotonic_ns': 100+i,
                       'bytes_b64': base64.b64encode(value).decode()})
    events.append({'request': 1, 'kind': 'complete', 'monotonic_ns': 1000})
    return '\n'.join(json.dumps(event) for event in events)


class VariantTests(unittest.TestCase):
    def setUp(self):
        self.source = [frame({'role': 'assistant'}), frame({'reasoning': 'real'}),
                       frame({'content': 'answer'}), b'data: [DONE]\n\n']

    def test_real_byte_modes_and_timing_populations(self):
        for mode, population in [('answer-first', 'answer_before_reasoning'), ('same-event', 'same_event')]:
            forwarded = sse_reorder.transform(self.source, mode)
            result = checks.verify_source_and_collect(captured(self.source, forwarded, mode), mode)
            self.assertEqual(result[population], 1)
            self.assertEqual(result['role'], 1)

    def test_fabricated_or_missing_content_rejected(self):
        forwarded = sse_reorder.transform(self.source, 'answer-first')
        forwarded[1] = frame({'content': 'fabricated'})
        with self.assertRaises(ValueError):
            checks.verify_source_and_collect(captured(self.source, forwarded, 'answer-first'), 'answer-first')
        with self.assertRaises(ValueError):
            checks.verify_source_and_collect(captured(self.source, self.source[:-1]), None)

    def test_tool_metadata_is_not_output(self):
        source = [self.source[0], frame({'tool_calls': [{'function': {'name': 'f', 'arguments': ''}}]}),
                  frame({'tool_calls': [{'function': {'arguments': '{'}}]}), self.source[-1]]
        self.assertEqual(checks.verify_source_and_collect(captured(source, source), None)['tool'], 1)
        with self.assertRaises(ValueError): checks.verify_source_and_collect(captured(source[:2] + source[-1:], source[:2] + source[-1:]), None)


if __name__ == '__main__':
    unittest.main()
