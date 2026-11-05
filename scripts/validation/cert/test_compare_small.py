# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""CPU-only contract tests; synthetic data never constitutes GPU evidence."""
import base64
import json
import tempfile
import unittest
from pathlib import Path

import compare_small


class SmallComparisonTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.a, self.b = self.root / "base", self.root / "candidate"
        self.base_sha, self.candidate_sha = "a" * 64, "b" * 64
        for workload, concurrency in compare_small.POINTS:
            for root, digest in ((self.a, self.base_sha), (self.b, self.candidate_sha)):
                cell = root / workload / f"c{concurrency}"
                cell.mkdir(parents=True)
                meta = {"model": "Qwen/Qwen3-8B", "model_revision": "1" * 40,
                        "image": "vllm/image@sha256:" + "2" * 64, "workload": workload,
                        "prompt_sha256": "3" * 64, "concurrency": concurrency,
                        "max_tokens": 128 if workload == "W1" else 512, "seed": 42,
                        "temperature": 0, "thinking": False, "warmup_seconds": 30,
                        "measure_seconds": 180, "binary_sha256": digest}
                (cell / "point.json").write_text(json.dumps(meta))
                rows = [{"schema_version": "metrum-ai-bench-cli.request.v5", "phase": "measure",
                         "send_offset_s": i, "error": None, "completion_tokens": 4,
                         "usage_completion_tokens_present": True, "latency_s": 2.0, "ttft_s": 1.0,
                         "ttft_visible_answer_s": 1.0} for i in (0, 1)]
                rows.append({"schema_version": "metrum-ai-bench-cli.summary.v5", "attempted": 2})
                (cell / "requests.jsonl").write_text("\n".join(map(json.dumps, rows)))
                sse = (b'data: {"choices":[{"delta":{"content":"answer"}}]}\n\n'
                       b'data: {"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":4}}\n\ndata: [DONE]\n\n')
                events = []
                for index in range(2):
                    events.extend([{"request": index, "kind": "chunk", "bytes_b64": base64.b64encode(sse).decode()},
                                   {"request": index, "kind": "complete"}])
                (cell / "trace.jsonl").write_text("\n".join(map(json.dumps, events)))

    def verdict(self, floors=None):
        return compare_small.compare(self.a, self.b, self.base_sha, self.candidate_sha, floors or {})

    def test_six_complete_matching_points(self):
        result = self.verdict()
        self.assertEqual("PASS", result["result"])
        self.assertEqual(6, len(result["points"]))
        self.assertAlmostEqual(8 / 3, result["points"][0]["baseline"]["legacy_output_tokens_per_second"])

    def test_missing_point_and_incomplete_usage_fail(self):
        (self.b / "W1" / "c1" / "trace.jsonl").unlink()
        self.assertEqual("FAIL", self.verdict()["result"])
        (self.b / "W2" / "c8" / "requests.jsonl").write_text('{}\n')
        self.assertEqual("FAIL", self.verdict()["points"][4]["result"])

    def test_noise_floor_and_pin_mismatch(self):
        path = self.b / "W1" / "c1" / "requests.jsonl"
        path.write_text(path.read_text().replace('"latency_s": 2.0', '"latency_s": 2.2'))
        self.assertEqual("FAIL", self.verdict()["points"][0]["result"])
        self.assertEqual("PASS", self.verdict({"W1/c1": {"legacy_output_tokens_per_second": 20, "tpot_p50_ms": 25}})["points"][0]["result"])
        meta = self.b / "W2" / "c32" / "point.json"
        value = json.loads(meta.read_text()); value["prompt_sha256"] = "4" * 64
        meta.write_text(json.dumps(value))
        self.assertEqual("FAIL", self.verdict()["points"][-1]["result"])


if __name__ == "__main__":
    unittest.main()
