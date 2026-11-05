# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""CPU-only tests of window and SLO population math; not GPU evidence."""
import base64
import json
import tempfile
import unittest
from pathlib import Path

import g3_checks
import g3_runner


class G3ChecksTests(unittest.TestCase):
    def fixture(self):
        def row(seq, start, latency, answered, outcome, error):
            return {"schema_version": "metrum-ai-bench-cli.request.v5", "phase": "measure", "seq": seq,
                    "send_offset_s": start, "latency_s": latency, "queue_delay_s": 0,
                    "answer_completed": answered, "outcome": outcome, "error": error,
                    "usage_completion_tokens_present": False}
        rows = [row(0, 0, 2, True, "answer", None), row(1, 0.5, 3, True, "answer", None),
                row(2, 1, 1, False, "reasoning_truncated", None),
                row(3, 1.2, 0.4, False, "error", {"kind": "connect"})]
        window = 3.5
        rows.append({"schema_version": "metrum-ai-bench-cli.summary.v5", "attempted": 4, "window_seconds": window,
                     "completion_tokens_total": None, "completion_tokens_per_second": None,
                     "answered_requests": 2, "requests_per_second": 3 / window,
                     "answered_requests_per_second": 2 / window,
                     "goodput": {"count": 1, "requests_per_second": 1 / window, "thresholds_s": {"e2e": 2.5}}})
        events = []
        for row in rows[:-1]:
            start = int((10 + row["send_offset_s"]) * 1e9)
            stop = int((10 + row["send_offset_s"] + row["latency_s"]) * 1e9)
            events.append({"request": row["seq"], "kind": "send", "monotonic_ns": start})
            if row["outcome"] == "answer":
                frame = b'data: {"choices":[{"delta":{"content":"answer"}}]}\n\n'
                events.append({"request": row["seq"], "kind": "chunk", "monotonic_ns": start + 1, "bytes_b64": base64.b64encode(frame).decode()})
            elif row["outcome"] == "reasoning_truncated":
                frame = b'data: {"choices":[{"delta":{"reasoning_content":"r"}}]}\n\ndata: {"choices":[{"delta":{},"finish_reason":"length"}]}\n\n'
                events.append({"request": row["seq"], "kind": "chunk", "monotonic_ns": start + 1, "bytes_b64": base64.b64encode(frame).decode()})
            events.append({"request": row["seq"], "kind": "error" if row["error"] else "complete", "monotonic_ns": stop})
        return "\n".join(map(json.dumps, rows)), "\n".join(map(json.dumps, events))

    def test_window_and_order(self):
        data, trace = self.fixture()
        self.assertEqual(1, g3_checks.verify_window(data, trace, "error-first")["truncated"])
        with self.assertRaises(ValueError):
            g3_checks.verify_window(data, trace, "error-last")
        with self.assertRaises(ValueError):
            g3_checks.verify_window(data, trace.replace('13500000000', '14500000000'), "error-first")
        with self.assertRaises(ValueError):
            g3_checks.verify_window(data, trace.replace('"kind": "error"', '"kind": "complete"'), "error-first")

    def test_complete_usage_control_requires_real_totals_and_rates(self):
        data, trace = self.fixture()
        rows = [json.loads(line) for line in data.splitlines()]
        rows = rows[:3] + [rows[-1]]
        for row in rows[:-1]:
            row.update({"usage_prompt_tokens_present": True, "usage_completion_tokens_present": True,
                        "prompt_tokens": 2, "completion_tokens": 3})
        rows[-1].update({"attempted": 3, "prompt_tokens_total": 6, "completion_tokens_total": 9,
                         "input_tokens_per_second": 6 / 3.5, "completion_tokens_per_second": 9 / 3.5,
                         "total_tokens_per_second": 15 / 3.5})
        result = []
        for event in map(json.loads, trace.splitlines()):
            if event["request"] == 3:
                continue
            if event["kind"] == "complete":
                frame = b'data: {"choices":[],"usage":{"prompt_tokens":2,"completion_tokens":3}}\n\n'
                result.append({"request": event["request"], "kind": "chunk", "monotonic_ns": event["monotonic_ns"] - 1,
                               "bytes_b64": base64.b64encode(frame).decode()})
            result.append(event)
        data = "\n".join(map(json.dumps, rows)); trace = "\n".join(map(json.dumps, result))
        self.assertEqual(9, g3_checks.verify_complete_control(data, trace)["tokens"]["completion_tokens"])
        with self.assertRaises(ValueError):
            g3_checks.verify_complete_control(data.replace('"completion_tokens_total": 9', '"completion_tokens_total": 8'), trace)

    def test_interrupt_trigger_requires_active_stream_and_population(self):
        with tempfile.TemporaryDirectory() as temp:
            trace, data = Path(temp) / "trace", Path(temp) / "data"
            stream = [{"request": 1, "kind": "chunk"}, {"request": 2, "kind": "chunk"}]
            trace.write_text("\n".join(map(json.dumps, stream)))
            data.write_text("")
            self.assertTrue(g3_runner.ready_to_interrupt("error-first", trace, data))
            self.assertFalse(g3_runner.ready_to_interrupt("error-last", trace, data))
            rows = [{"schema_version": "test.request.v5", "answer_completed": True, "error": None},
                    {"schema_version": "test.request.v5", "outcome": "reasoning_truncated"}]
            data.write_text("\n".join(map(json.dumps, rows)))
            self.assertTrue(g3_runner.ready_to_interrupt("error-last", trace, data))
            stream += [{"request": 1, "kind": "complete"}, {"request": 2, "kind": "complete"}]
            trace.write_text("\n".join(map(json.dumps, stream)))
            self.assertFalse(g3_runner.ready_to_interrupt("error-last", trace, data))

    def test_goodput_and_unauthorized_incomplete_usage(self):
        data, trace = self.fixture()
        self.assertEqual(1, g3_checks.verify_goodput(data, trace, 2.5)["answered_pass"])
        with self.assertRaises(ValueError):
            g3_checks.verify_goodput(data, trace, 10)
        with self.assertRaises(ValueError):
            g3_checks.verify_window(data.replace('"completion_tokens_total": null', '"completion_tokens_total": 3'), trace, "error-first")


if __name__ == "__main__":
    unittest.main()
