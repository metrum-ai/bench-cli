# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic fixtures exercise byte transformations only, not GPU cases."""
import json
import unittest

import fault_transform


class FaultTransformTests(unittest.TestCase):
    def event(self, frame):
        return json.loads(next(line[5:].strip() for line in frame.splitlines() if line.startswith(b"data:")))

    def test_usage_and_split_are_removed_only_when_present(self):
        frame = b'data: {"choices":[],"usage":{"prompt_tokens":2,"completion_tokens":5,"completion_tokens_details":{"reasoning_tokens":3}}}\n\n'
        forwarded, changed = fault_transform.transform(frame, "remove_usage")
        self.assertTrue(changed)
        self.assertNotIn("usage", self.event(forwarded))
        forwarded, changed = fault_transform.transform(frame, "remove_reasoning_split")
        self.assertTrue(changed)
        usage = self.event(forwarded)["usage"]
        self.assertEqual(5, usage["completion_tokens"])
        self.assertNotIn("reasoning_tokens", usage["completion_tokens_details"])
        unrelated = b'data: {"choices":[{"delta":{"content":"a"}}]}\n\n'
        self.assertEqual((unrelated, False), fault_transform.transform(unrelated, "remove_usage"))

    def test_finish_duplicate_removal_and_nonlength(self):
        frame = b'data: {"choices":[{"index":0,"finish_reason":"length","delta":{}}]}\r\n\r\n'
        removed, changed = fault_transform.transform(frame, "remove_finish")
        self.assertTrue(changed)
        self.assertIsNone(self.event(removed)["choices"][0]["finish_reason"])
        duplicate, changed = fault_transform.transform(frame, "duplicate_finish")
        self.assertTrue(changed)
        self.assertEqual(frame + frame, duplicate)
        replaced, changed = fault_transform.transform(frame, "nonlength_finish")
        self.assertTrue(changed)
        self.assertEqual("stop", self.event(replaced)["choices"][0]["finish_reason"])

    def test_remove_output_and_both_terminal_markers(self):
        token = b'data: {"choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"r","content":"a"}}]}\n\n'
        changed, applied = fault_transform.transform(token, "remove_output")
        self.assertTrue(applied)
        self.assertEqual({"role": "assistant"}, self.event(changed)["choices"][0]["delta"])
        finish = b'data: {"choices":[{"finish_reason":"length","delta":{}}]}\n\n'
        changed, applied = fault_transform.transform(finish, "remove_finish_and_done")
        self.assertTrue(applied)
        self.assertIsNone(self.event(changed)["choices"][0]["finish_reason"])
        self.assertEqual((b"", True), fault_transform.transform(b'data: [DONE]\n\n', "remove_finish_and_done"))

    def test_corrupt_and_done_are_fail_closed(self):
        frame = b'data: {"choices":[{"delta":{"content":"actual"}}]}\n\n'
        broken, changed = fault_transform.transform(frame, "corrupt")
        self.assertTrue(changed)
        with self.assertRaises(ValueError):
            self.event(broken)
        done = b'data: [DONE]\n\n'
        self.assertEqual((done, False), fault_transform.transform(done, "corrupt"))
        with self.assertRaises(ValueError):
            fault_transform.transform(frame, "unknown")


if __name__ == "__main__":
    unittest.main()
