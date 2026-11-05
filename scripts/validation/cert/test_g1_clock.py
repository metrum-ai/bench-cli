# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline stream-clock fixtures, never GPU evidence."""
import base64
import json
import unittest

import g1_clock_checks


def trace(frames):
    events = [{'request': 1, 'kind': 'send', 'monotonic_ns': 0}]
    for i, obj in enumerate(frames, 1):
        raw = ('data: ' + json.dumps(obj) + '\n\n').encode()
        events.append({'request': 1, 'kind': 'chunk', 'monotonic_ns': i * 1000,
                       'bytes_b64': base64.b64encode(raw).decode()})
    events.append({'request': 1, 'kind': 'complete', 'monotonic_ns': 9000})
    return '\n'.join(json.dumps(event) for event in events)


class ClockTests(unittest.TestCase):
    def test_tool_metadata_is_not_a_token(self):
        frames = [
            {'choices': [{'index': 0, 'delta': {'role': 'assistant'}}]},
            {'choices': [{'index': 0, 'delta': {'tool_calls': [{'function': {'name': 'f', 'arguments': ''}}]}}]},
            {'choices': [{'index': 1, 'delta': {'content': 'not selected'}}]},
            {'choices': [{'index': 0, 'delta': {'tool_calls': [{'function': {'arguments': '{'}}]}}]},
        ]
        observed = g1_clock_checks.stream_clocks(trace(frames))[0]
        self.assertEqual(observed['first'], 4000)
        self.assertEqual(observed['visible'], 4000)

    def test_indexless_choices_use_position_and_legacy_text(self):
        frames = [{'choices': [{'delta': {'role': 'assistant'}}, {'delta': {'content': 'not selected'}}]},
                  {'choices': [{'text': 'selected'}]}]
        observed = g1_clock_checks.stream_clocks(trace(frames))[0]
        self.assertEqual(observed['first'], 2000)
        self.assertEqual(observed['visible'], 2000)

    def test_reasoning_before_tool(self):
        frames = [{'choices': [{'index': 0, 'delta': {'reasoning': 'thinking'}}]},
                  {'choices': [{'index': 0, 'delta': {'function_call': {'arguments': 'ok'}}}]}]
        observed = g1_clock_checks.stream_clocks(trace(frames))[0]
        self.assertTrue(observed['reasoning_seen'])
        self.assertEqual(observed['first'], 1000)
        self.assertEqual(observed['visible'], 2000)


if __name__ == '__main__':
    unittest.main()
