# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic terminal-fault oracle tests; not GPU evidence."""
import base64
import json
import unittest

import fault_transform
import g2_terminal_checks


class TerminalChecksTests(unittest.TestCase):
    def fixture(self, mode):
        frames = [b'data: {"choices":[{"index":0,"delta":{"reasoning_content":"real"}}]}\n\n',
                  b'data: {"choices":[{"index":0,"delta":{},"finish_reason":"length"}]}\n\n',
                  b'data: {"choices":[],"usage":{"prompt_tokens":2,"completion_tokens":3}}\n\n',
                  b'data: [DONE]\n\n']
        output = []
        changes = 0
        for frame in frames:
            result, changed = fault_transform.transform(frame, mode)
            output.append(result); changes += int(changed)
        b64 = lambda value: base64.b64encode(value).decode()
        events = [{"request": 1, "kind": "chunk", "bytes_b64": b64(b"".join(frames))},
                  {"request": 1, "kind": "forwarded_chunk", "bytes_b64": b64(b"".join(output))}]
        events += [{"request": 1, "kind": "transformation", "mode": mode} for _ in range(changes)]
        events.append({"request": 1, "kind": "complete"})
        error = None if mode == "duplicate_finish" else {"kind": "stream_truncated" if mode == "remove_finish_and_done" else "no_output_token"}
        outcome = "reasoning_truncated" if error is None else ("error" if mode == "remove_finish_and_done" else "no_output_token")
        request = {"schema_version": "bench.request.v5", "phase": "measure", "error": error, "outcome": outcome,
                   "answer_completed": False, "ttft_s": .1, "prompt_tokens": 2, "completion_tokens": 3,
                   "usage_prompt_tokens_present": True, "usage_completion_tokens_present": True}
        summary = {"schema_version": "bench.summary.v5", "attempted": 1, "errors": int(error is not None),
                   "ttft_coverage": {"reasoning_truncated": int(error is None)}}
        return "\n".join(map(json.dumps, [request, summary])), "\n".join(map(json.dumps, events)), int(error is not None)

    def test_each_terminal_fault_and_missing_transformation(self):
        for mode in ("remove_output", "remove_finish_and_done", "nonlength_finish", "duplicate_finish"):
            with self.subTest(mode=mode):
                data, trace, code = self.fixture(mode)
                self.assertEqual(mode, g2_terminal_checks.fault_cell(data, trace, code, mode, 1)["mode"])
                with self.assertRaises(ValueError):
                    g2_terminal_checks.fault_cell(data, trace.replace('"mode": "' + mode + '"', '"mode": "wrong"'), code, mode, 1)
                with self.assertRaises(ValueError):
                    g2_terminal_checks.fault_cell(data, trace, 1 - code, mode, 1)


if __name__ == "__main__":
    unittest.main()
