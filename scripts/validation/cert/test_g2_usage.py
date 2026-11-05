# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""CPU-only fault-evidence checks; no GPU or serving claims."""
import base64
import json
import unittest

import fault_transform
import g2_usage_checks


class UsageEvidenceTests(unittest.TestCase):
    def fixture(self, mode):
        frames = [b'data: {"choices":[{"index":0,"delta":{"reasoning_content":"r"}}]}\n\n',
                  b'data: {"choices":[{"index":0,"delta":{"content":"answer"}}]}\n\n',
                  b'data: {"choices":[],"usage":{"prompt_tokens":2,"completion_tokens":3,"completion_tokens_details":{"reasoning_tokens":1}}}\n\n',
                  b'data: [DONE]\n\n']
        events = [{"request": 1, "kind": "send", "monotonic_ns": 1000000000}]
        for index, frame in enumerate(frames):
            stamp = 1100000000 + index * 50000000
            events.append({"request": 1, "kind": "chunk", "monotonic_ns": stamp, "bytes_b64": base64.b64encode(frame).decode()})
            output, changed = (frame, False) if mode == "control" else fault_transform.transform(frame, mode)
            if changed:
                events.append({"request": 1, "kind": "transformation", "monotonic_ns": stamp, "mode": mode})
            events.append({"request": 1, "kind": "forwarded_chunk", "monotonic_ns": stamp, "bytes_b64": base64.b64encode(output).decode()})
        events.append({"request": 1, "kind": "complete", "monotonic_ns": 1500000000})
        row = {"schema_version": "metrum-ai-bench-cli.request.v5", "phase": "measure", "seq": 0,
               "error": None, "latency_s": 0.5, "ttft_s": 0.1, "ttft_source": "stream", "ttft_visible_answer_s": 0.15,
               "usage_prompt_tokens_present": mode != "remove_usage", "usage_completion_tokens_present": mode != "remove_usage",
               "reasoning_tokens": None if mode != "control" else 1}
        summary = {"schema_version": "metrum-ai-bench-cli.summary.v5", "input_tokens_per_second": None,
                   "completion_tokens_per_second": None, "total_tokens_per_second": None,
                   "non_reasoning_completion_tokens_per_second": None}
        return "\n".join(map(json.dumps, [row, summary])), "\n".join(map(json.dumps, events))

    def test_each_mode_and_missing_proof(self):
        for mode in ("control", "remove_usage", "remove_reasoning_split"):
            with self.subTest(mode=mode):
                data, trace = self.fixture(mode)
                self.assertEqual(mode, g2_usage_checks.evaluate(data, trace, 0, mode, 1)["mode"])
                if mode != "control":
                    with self.assertRaises(ValueError):
                        g2_usage_checks.evaluate(data, trace.replace('"kind": "transformation"', '"kind": "removed"'), 0, mode, 1)


if __name__ == "__main__":
    unittest.main()
