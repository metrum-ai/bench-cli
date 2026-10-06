#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Metrum AI Bench CLI parity harness tests for count_points.py.

Run: python3 -m unittest discover -s scripts/parity

Issue #245: a distribution statistic that is null for small n (std for
n < 2) must count the same as a numeric one, on the Bench and AIPerf sides,
so the values column does not jitter between identical runs. An empty
distribution (n == 0) must not credit its undefined statistics as values.
"""

from __future__ import annotations

import copy
import json
import tempfile
import unittest
from pathlib import Path

import count_points as cp


def empty_dist() -> dict:
    """A Bench DistSummary with no samples: every statistic but n is null."""
    d = {k: None for k in ("min", "max", "avg", "std", "mad", "p50", "p90", "p95", "p99")}
    return {"n": 0, **d, "percentile_method": "type7", "p99_unreliable": True}


def dist(n: int, std: float | None) -> dict:
    """A Bench DistSummary as summary.v3 serializes it."""
    return {
        "n": n, "min": 1.0, "max": 2.0, "avg": 1.5, "std": std, "mad": 0.5,
        "p50": 1.5, "p90": 1.9, "p95": 1.95, "p99": 1.99,
        "percentile_method": "type7", "p99_unreliable": True,
    }


def bench_log(tmp: Path, std: float | None, name: str) -> Path:
    """A minimal Bench --data-log: two paced measured requests and a summary."""
    rows = [
        {"schema_version": "metrum-ai-bench-cli.request.v3", "phase": "measure",
         "seq": i, "ttft_s": 0.1, "latency_s": 0.5, "output_tokens": 10}
        for i in range(2)
    ]
    rows.append({
        "schema_version": "metrum-ai-bench-cli.summary.v3",
        "attempted": 2,
        "optional_scalar": None,
        "latency_s": dist(2, 0.1),
        "throughput_bins_rps": dist(1, std),
        "queue_delay_s": empty_dist(),
        "goodput": {"rate": 1.0, "thresholds_s": {"ttft": 0.2, "e2e": None}},
        "config": {"concurrency": 4},
    })
    path = tmp / name
    path.write_text("".join(json.dumps(r) + "\n" for r in rows), encoding="utf-8")
    return path


AIPERF_SUMMARY = {
    "aiperf_version": "0.13.0",
    "request_latency": {"unit": "ms", "avg": 500.0, "p1": 490.0, "p50": 500.0,
                        "p99": 510.0, "min": 480.0, "max": 520.0, "std": 5.0,
                        "count": 2, "sum": 1000.0},
    "request_throughput": {"unit": "requests/sec", "avg": 4.0},
    "empty_block": {"unit": "ms", "avg": None},
    "reasoning_token_count": {"unit": "tokens", "avg": None, "p50": None, "p99": None,
                              "min": None, "max": None, "std": None, "count": 0,
                              "sum": None},
    "input_config": {"unit": "n/a", "x": 1},
}


def aiperf_dir(tmp: Path, std: float | None, name: str) -> Path:
    art = tmp / name
    art.mkdir()
    summary = copy.deepcopy(AIPERF_SUMMARY)
    summary["request_latency"]["std"] = std
    (art / "profile_export_aiperf.json").write_text(json.dumps(summary), encoding="utf-8")
    rec = {"metadata": {"benchmark_phase": "profiling"},
           "metrics": {"time_to_first_token": {"value": 100.0},
                       "request_latency": {"value": 500.0}}}
    (art / "profile_export.jsonl").write_text(json.dumps(rec) + "\n", encoding="utf-8")
    return art


class NullableStatisticTest(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def test_bench_null_std_counts_same_as_numeric(self) -> None:
        numeric = cp.count_bench(bench_log(self.tmp, 0.25, "a.jsonl"), 0.9)
        null = cp.count_bench(bench_log(self.tmp, None, "b.jsonl"), 0.9)
        self.assertGreater(numeric["values"], 0)
        for key in ("quantities", "values", "dists", "blocks", "per_request"):
            self.assertEqual(numeric[key], null[key], key)
        self.assertEqual(numeric["null_values"], 0)
        self.assertEqual(null["null_values"], 1)

    def test_bench_values_breakdown(self) -> None:
        out = cp.count_bench(bench_log(self.tmp, None, "a.jsonl"), 0.9)
        # attempted (1) + two non-empty DistSummary with 10 slots each
        # (strings and booleans excluded) + the empty queue_delay_s, which
        # keeps only its n (1) + goodput leftovers rate and thresholds_s.ttft
        # (2; a null outside a distribution never counts). optional_scalar
        # is null and config is metadata.
        self.assertEqual(out["quantities"], 5)
        self.assertEqual(out["dists"], 3)
        self.assertEqual(out["blocks"], 1)
        self.assertEqual(out["values"], 1 + 10 + 10 + 1 + 2)
        self.assertEqual(out["null_values"], 1)
        self.assertEqual(out["empty_dists"], 1)
        self.assertEqual(out["empty_dist_names"], ["queue_delay_s"])

    def test_empty_dist_counts_only_n(self) -> None:
        self.assertEqual(cp.dist_slots(empty_dist(), "n"), [0.0])
        self.assertEqual(len(cp.dist_slots(dist(1, None), "n")), 10)
        self.assertEqual(len(cp.dist_slots(dist(1, 0.5), "n")), 10)
        # A block without the size key (an AIPerf scalar) is never empty.
        self.assertEqual(cp.dist_slots({"avg": 1.0, "std": None}, "count"), [1.0, None])

    def test_aiperf_null_std_counts_same_as_numeric(self) -> None:
        numeric = cp.count_aiperf(aiperf_dir(self.tmp, 5.0, "a"), 0.9)
        null = cp.count_aiperf(aiperf_dir(self.tmp, None, "b"), 0.9)
        self.assertGreater(numeric["values"], 0)
        for key in ("quantities", "values", "dists", "blocks", "per_request"):
            self.assertEqual(numeric[key], null[key], key)
        # request_latency 9 slots + request_throughput 1 + the empty
        # reasoning_token_count, which keeps only its count (1); empty_block
        # holds no number at all and input_config is metadata.
        self.assertEqual(numeric["quantities"], 3)
        self.assertEqual(numeric["values"], 11)
        self.assertEqual(numeric["empty_dists"], 1)
        self.assertEqual(numeric["null_values"], 0)
        self.assertEqual(null["null_values"], 1)

    def test_duplicates_keep_null_slots(self) -> None:
        groups = cp.find_duplicates({
            "a": [1.0, None, 2.0],
            "b": [1.0, None, 2.0],
            "c": [1.0, 0.0, 2.0],
            "lone": [None],
        })
        self.assertEqual(groups, [["a", "b"]])

    def test_table_shows_null_values(self) -> None:
        counts = self.tmp / "counts.jsonl"
        out = cp.count_bench(bench_log(self.tmp, None, "a.jsonl"), 0.9)
        counts.write_text(json.dumps({"scenario": "plain", **out}) + "\n", encoding="utf-8")
        text = cp.table(counts)
        self.assertIn("null values", text)
        self.assertIn(f"| {out['quantities']} / {out['values']} / {out['per_request']}", text)


if __name__ == "__main__":
    unittest.main()
