# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic reference alignment tests; no GPU result is implied."""
import base64
import json
import unittest

import comp_reference_checks as check
import trace_proxy


class ReferenceChecksTests(unittest.TestCase):
    def fixture(self):
        model = "Qwen/Qwen3-8B"
        order, events, rows = [], [], []
        def chunk(i, timestamp, value):
            data = b'data: ' + json.dumps(value).encode() + b'\n\n'
            return {"request": i, "kind": "chunk", "monotonic_ns": timestamp, "bytes_b64": base64.b64encode(data).decode()}
        for i in range(64):
            prompt = f"question-{i}"
            body = {"messages": [{"role": "user", "content": prompt}], "model": model,
                    "max_tokens": 4096, "temperature": 0, "seed": 42, "stream": True, "chat_template_kwargs": {"enable_thinking": True}}
            identity = trace_proxy.request_identity(json.dumps(body).encode())
            order.append({"seq": i, "prompt_sha256_16": identity["prompt_sha256"][:16], "prompt_sha256": identity["prompt_sha256"]})
            sent = 1_000_000_000 + i * 2_000_000_000
            events += [{"request": i, "kind": "request_identity", **identity},
                       {"request": i, "kind": "send", "monotonic_ns": sent},
                       chunk(i, sent + 100_000_000, {"choices": [{"index": 0, "delta": {"role": "assistant", "content": ""}}]}),
                       chunk(i, sent + 200_000_000, {"choices": [{"index": 0, "delta": {"reasoning_content": "r"}}]}),
                       chunk(i, sent + 600_000_000, {"choices": [{"index": 0, "delta": {"content": "a"}}]}),
                       chunk(i, sent + 700_000_000, {"choices": [], "usage": {"prompt_tokens": 2, "completion_tokens": 3}}),
                       {"request": i, "kind": "complete", "monotonic_ns": sent + 1_000_000_000}]
            rows.append({"schema_version": "bench.request.v5", "phase": "measure", "seq": i, "error": None,
                         "answer_completed": True, "usage_prompt_tokens_present": True, "usage_completion_tokens_present": True,
                         "prompt_tokens": 2, "completion_tokens": 3, "ttft_s": .2})
        rows.append({"schema_version": "bench.summary.v5", "attempted": 64, "window_seconds": 127,
                     "completion_tokens_total": 192, "completion_tokens_per_second": 192 / 127})
        ref = {"completed": 64, "failed": 0, "input_lens": [2] * 64, "output_lens": [3] * 64,
               "ttfts": [.1] * 64, "generated_texts": ["a"] * 64, "errors": [""] * 64,
               "duration": 127, "total_output_tokens": 192, "output_throughput": 192 / 127}
        return "\n".join(map(json.dumps, rows)), "\n".join(map(json.dumps, events)), ref, order

    def test_two_definitions_are_distinct_and_development_only(self):
        data, trace, ref, order = self.fixture()
        result = check.compare(data, trace, ref, trace, order, 'Qwen/Qwen3-8B', 4096)
        self.assertTrue(result['reference_ttft_aligned'])
        self.assertTrue(result['throughput_within_10_percent'])
        self.assertAlmostEqual(-.1, result['first_choices_minus_first_any_median_s'])
        gaps = check.observed_clock_gaps(trace)
        self.assertEqual(64, gaps['paired_streams'])
        self.assertAlmostEqual(.1, gaps['first_choices_p50_s'])
        self.assertAlmostEqual(.2, gaps['first_any_p50_s'])
        self.assertAlmostEqual(-.1, gaps['first_choices_minus_first_any_median_s'])
        self.assertIn('not release certification', result['criteria_status'])
        wrong = {**ref, 'ttfts': [.2] * 64}
        with self.assertRaises(ValueError):
            check.compare(data, trace, wrong, trace, order, 'Qwen/Qwen3-8B', 4096)
        bad_order = list(order); bad_order[0] = {**bad_order[0], 'prompt_sha256': '0' * 64}
        with self.assertRaises(ValueError):
            check.compare(data, trace, ref, trace, bad_order, 'Qwen/Qwen3-8B', 4096)

    def test_request_text_normalization_and_settings(self):
        body = {"messages": [{"role": "user", "content": "hi"}], "model": "m", "max_tokens": 5,
                "chat_template_kwargs": {"enable_thinking": True}, "temperature": 0}
        one = trace_proxy.request_identity(json.dumps(body).encode())
        body['messages'][0]['content'] = [{'type': 'text', 'text': 'hi'}]
        body['max_completion_tokens'] = body.pop('max_tokens')
        self.assertEqual(one, trace_proxy.request_identity(json.dumps(body).encode()))
        body['messages'][0]['content'] = [{'type': 'image_url', 'image_url': 'private'}]
        with self.assertRaises(ValueError):
            trace_proxy.request_identity(json.dumps(body).encode())


if __name__ == '__main__':
    unittest.main()
