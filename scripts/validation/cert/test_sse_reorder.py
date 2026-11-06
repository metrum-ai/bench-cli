# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic transform unit tests only: these are never GPU evidence."""
import json
import unittest

import sse_reorder


def frame(delta, **kwargs):
    value = {'id': 'source', 'choices': [{'index': 0, 'delta': delta, 'finish_reason': None}], **kwargs}
    return b'data: ' + json.dumps(value, separators=(',', ':')).encode() + b'\n\n'


class ReorderTests(unittest.TestCase):
    def setUp(self):
        self.role = frame({'role': 'assistant'})
        self.reason1 = frame({'reasoning': 'thought'})
        self.reason2 = frame({'reasoning': 'more'})
        self.answer = frame({'content': 'answer'})
        self.done = b'data: [DONE]\n\n'
        self.source = [self.role, self.reason1, self.reason2, self.answer, self.done]

    def test_answer_first_moves_real_frame_only(self):
        result = sse_reorder.transform(self.source, 'answer-first')
        self.assertEqual(result, [self.role, self.answer, self.reason1, self.reason2, self.done])

    def test_same_event_combines_original_deltas_only(self):
        result = sse_reorder.transform(self.source, 'same-event')
        self.assertEqual(result[0], self.role)
        self.assertEqual(sse_reorder.decode(result[1])['choices'][0]['delta'], {'content': 'answer', 'reasoning': 'thought'})
        self.assertEqual(result[2:], [self.reason2, self.done])

    def test_unselected_first_moves_actual_nonselected_token(self):
        other = b'data: {"choices":[{"index":1,"delta":{"content":"other"}}]}\n\n'
        result = sse_reorder.transform([self.role, self.answer, other, self.done], 'unselected-first')
        self.assertEqual(result, [self.role, other, self.answer, self.done])
        with self.assertRaises(ValueError):
            sse_reorder.transform([self.role, self.answer, self.done], 'unselected-first')

    def test_missing_reasoning_or_done_rejected(self):
        with self.assertRaises(ValueError): sse_reorder.transform([self.role, self.answer, self.done], 'answer-first')
        with self.assertRaises(ValueError): sse_reorder.transform(self.source[:-1], 'same-event')

    def test_ambiguous_or_multi_choice_rejected(self):
        with self.assertRaises(ValueError): sse_reorder.transform([frame({'reasoning': 'r', 'content': 'a'}), self.done], 'same-event')
        multichoice = b'data: {"choices":[{"index":0,"delta":{"reasoning":"r"}},{"index":1,"delta":{}}]}\n\n'
        with self.assertRaises(ValueError): sse_reorder.transform([multichoice, self.answer, self.done], 'answer-first')

    def test_output_detection_includes_tool_arguments_and_legacy_text(self):
        metadata = frame({'tool_calls': [{'function': {'name': 'f', 'arguments': ''}}]})
        tool = frame({'tool_calls': [{'function': {'arguments': '{"city":"x"}'}}]})
        legacy = b'data: {"id":"source","choices":[{"index":0,"delta":{},"text":"answer","finish_reason":null}]}\n\n'
        self.assertFalse(sse_reorder.has_selected_output([self.role, self.reason1, metadata, self.done]))
        for answer in (tool, legacy):
            self.assertTrue(sse_reorder.has_selected_output([self.role, self.reason1, answer, self.done]))
            for mode in ('answer-first', 'same-event'):
                self.assertEqual(len(sse_reorder.transform([self.role, self.reason1, answer, self.done], mode)),
                                 4 if mode == 'answer-first' else 3)
        with self.assertRaises(ValueError):
            sse_reorder.transform([self.role, self.reason1, metadata, tool, self.done], 'answer-first')

    def test_unpreserved_source_metadata_rejected(self):
        reason = json.loads(self.reason1[6:-2]); reason['choices'][0]['logprobs'] = {'unexpected': 1}
        unsafe = b'data: ' + json.dumps(reason).encode() + b'\n\n'
        with self.assertRaises(ValueError): sse_reorder.transform([unsafe, self.answer, self.done], 'same-event')


if __name__ == '__main__':
    unittest.main()
