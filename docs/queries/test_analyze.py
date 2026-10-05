#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Tests for analyze.py. Run: python3 -m unittest discover -s docs/queries

fixtures/sweep{5,3}.ndjson are recorded runs of metrum-ai-bench-cli-strategic
against metrum-ai-bench-cli-mock-server --telemetry-fixture (fixtures/record.sh).
Assertions on them hold for both stdout shapes: with `knee_detection` (#190)
and without it (older builds, legacy `knee` fallback).
"""

from __future__ import annotations

import contextlib
import copy
import io
import json
import os
import re
import unittest

import analyze

HERE = os.path.dirname(os.path.abspath(__file__))
FIXTURES = os.path.join(HERE, "fixtures")
ALL_SMI_PAGE = os.path.join(
    HERE, "..", "..", "scripts", "parity", "fixtures", "all-smi-fork-h100.prom"
)


def load_fixture(name):
    rows = analyze.load_rows(os.path.join(FIXTURES, f"{name}.ndjson"))
    with open(os.path.join(FIXTURES, f"{name}.stdout.json"), encoding="utf-8") as f:
        return rows, json.load(f)


def tele(metric, t_s, value, **labels):
    return {"kind": "telemetry", "run_id": "r", "t_ns": int(t_s * 1e9), "src": "s",
            "metric": metric, "labels": labels or {"gpu": "0"}, "value": value}


class Percentiles(unittest.TestCase):
    def test_type7_matches_numpy_linear(self):
        self.assertEqual(analyze.percentile_type7([4, 1, 3, 2], 50), 2.5)
        self.assertAlmostEqual(analyze.percentile_type7([1, 2, 3, 4], 95), 3.85)
        self.assertAlmostEqual(analyze.percentile_type7(list(range(1, 11)), 90), 9.1)
        self.assertEqual(analyze.percentile_type7([7.0], 95), 7.0)
        self.assertIsNone(analyze.percentile_type7([], 50))

    def test_histogram_quantile_interpolates(self):
        # 10 obs <= 0.1, 20 more <= 0.5, 10 more <= 1.0, none above.
        b = [(0.1, 10), (0.5, 30), (1.0, 40), (float("inf"), 40)]
        self.assertAlmostEqual(analyze.histogram_quantile(b, 0.5), 0.1 + 0.4 * 10 / 20)
        self.assertAlmostEqual(analyze.histogram_quantile(b, 0.25), 0.1)
        self.assertAlmostEqual(analyze.histogram_quantile(b, 0.95), 0.5 + 0.5 * 8 / 10)

    def test_histogram_quantile_edges(self):
        self.assertEqual(analyze.histogram_quantile([(1.0, 5), (float("inf"), 10)], 0.9), 1.0)
        self.assertIsNone(analyze.histogram_quantile([(1.0, 0), (float("inf"), 0)], 0.5))
        self.assertIsNone(analyze.histogram_quantile([], 0.5))

    def test_stage_histogram_uses_bucket_deltas(self):
        m = "vllm:time_to_first_token_seconds"
        rows = []
        for t, counts in ((0.0, (100, 100, 100)), (1.0, (100, 110, 120))):
            for le, c in zip(("0.1", "0.5", "+Inf"), counts):
                rows.append(tele(m + "_bucket", t, c, le=le, engine="0"))
        # Deltas: 0 <= 0.1, 10 <= 0.5, 20 total.
        self.assertAlmostEqual(analyze.stage_histogram_quantile(rows, m, 0.5), 0.1 + 0.4 * 10 / 10)
        self.assertEqual(analyze.stage_histogram_quantile(rows, m, 0.95), 0.5)


class DerivedMetrics(unittest.TestCase):
    def test_all_smi_names_exist_on_recorded_fork_page(self):
        with open(ALL_SMI_PAGE, encoding="utf-8") as f:
            page = set(re.findall(r"^# TYPE (\S+) ", f.read(), re.M))
        families = (analyze.GPU_UTIL + analyze.SM_ACTIVE + analyze.SM_OCCUPANCY
                    + analyze.TENSOR_ACTIVE + analyze.HOLLOW_UTIL)
        names = [m for m, _ in families if m.startswith("all_smi_")]
        self.assertEqual(len(names), 5)
        for name in names:
            self.assertIn(name, page)

    def test_all_smi_preferred_over_dcgm(self):
        rows = []
        for t in (0.0, 1.0):
            rows += [
                tele("all_smi_gpu_utilization", t, 80.0),
                tele("DCGM_FI_DEV_GPU_UTIL", t, 10.0),
                tele("all_smi_gpu_sm_active_ratio", t, 0.7),
                tele("DCGM_FI_PROF_SM_ACTIVE", t, 0.1),
                tele("all_smi_gpu_sm_occupancy", t, 0.2),
                tele("all_smi_gpu_tensor_active_ratio", t, 0.3),
                tele("all_smi_gpu_hollow_utilization_ratio", t, 0.05),
            ]
        d = analyze.derived_metrics(rows)
        self.assertAlmostEqual(d["gpu_util_mean"], 0.8)
        self.assertAlmostEqual(d["sm_active_p50"], 0.7)
        self.assertAlmostEqual(d["sm_occupancy_p50"], 0.2)
        self.assertAlmostEqual(d["tensor_active_p50"], 0.3)
        self.assertAlmostEqual(d["hollow_util_mean"], 0.05)
        self.assertEqual(d["sources"]["sm_active_p50"], "all_smi_gpu_sm_active_ratio")

    def test_dcgm_prof_fallbacks(self):
        rows = []
        for t, gr in ((0.0, 0.9), (1.0, 0.5)):
            rows += [
                tele("DCGM_FI_DEV_GPU_UTIL", t, 50.0),
                tele("DCGM_FI_PROF_SM_ACTIVE", t, 0.7),
                tele("DCGM_FI_PROF_SM_OCCUPANCY", t, 0.25),
                tele("DCGM_FI_PROF_PIPE_TENSOR_ACTIVE", t, 0.4),
                tele("DCGM_FI_PROF_GR_ENGINE_ACTIVE", t, gr),
            ]
        d = analyze.derived_metrics(rows)
        self.assertAlmostEqual(d["gpu_util_mean"], 0.5)
        self.assertAlmostEqual(d["sm_active_p50"], 0.7)
        self.assertAlmostEqual(d["sm_occupancy_p50"], 0.25)
        self.assertAlmostEqual(d["tensor_active_p50"], 0.4)
        # gr - sm per scrape, clamped: 0.2 then 0.0, time-weighted mean 0.1.
        self.assertAlmostEqual(d["hollow_util_mean"], 0.1)
        self.assertIn("DCGM_FI_PROF_GR_ENGINE_ACTIVE", d["sources"]["hollow_util_mean"])

    def test_absent_series_are_none_not_zero(self):
        d = analyze.derived_metrics([tele("all_smi_gpu_temperature_celsius", 0.0, 60.0)])
        for k in ("gpu_util_mean", "sm_active_p50", "sm_occupancy_p50", "tensor_active_p50",
                  "hollow_util_mean", "kv_cache_util_mean", "preemptions_delta"):
            self.assertIsNone(d[k], k)
            self.assertIsNone(d["sources"][k], k)

    def test_preemption_counter_reset_invalidates_stage(self):
        rows = [tele("vllm:num_preemptions_total", t, v) for t, v in ((0, 5), (1, 7), (2, 1))]
        self.assertIsNone(analyze.derived_metrics(rows)["preemptions_delta"])
        rows = [tele("vllm:num_preemptions_total", t, v) for t, v in ((0, 5), (1, 7), (2, 9))]
        self.assertEqual(analyze.derived_metrics(rows)["preemptions_delta"], 4)


class RecordedSweeps(unittest.TestCase):
    def test_sweep5_has_at_least_five_points(self):
        rows, stdout = load_fixture("sweep5")
        stages = [r for r in rows if r.get("kind") == "stage" and r.get("phase") == "measure"]
        self.assertGreaterEqual(len(stdout["points"]), analyze.KNEE_MIN_POINTS)
        self.assertEqual(len(stages), len(stdout["points"]))

    def test_sweep5_stage_metrics(self):
        rows, stdout = load_fixture("sweep5")
        res = analyze.analyze(rows, stdout)
        self.assertEqual(len(res["stages"]), 5)
        final = max(r["value"] for r in rows if r.get("metric") == "vllm:num_preemptions_total")
        self.assertLessEqual(sum(s["preemptions_delta"] for s in res["stages"]), final)
        for s in res["stages"]:
            self.assertEqual(s["sources"]["sm_active_p50"], "DCGM_FI_PROF_SM_ACTIVE")
            self.assertAlmostEqual(s["sm_active_p50"], 0.55)
            self.assertAlmostEqual(s["kv_cache_util_mean"], 0.25)
            self.assertTrue(0.0 <= s["gpu_util_mean"] <= 1.0)
            # The mock page has no GPM gauges: reported absent, not 0.
            self.assertIsNone(s["sm_occupancy_p50"])
            self.assertIsNone(s["tensor_active_p50"])
            self.assertIsNone(s["hollow_util_mean"])

    def test_sweep5_kv_at_knee_as_recorded(self):
        rows, stdout = load_fixture("sweep5")
        res = analyze.analyze(rows, stdout)
        det = stdout.get("knee_detection")
        if det is None:
            expected = stdout["knee"]["load"]
            self.assertEqual(res["knee"]["source"], "legacy_knee")
            self.assertIn("predates #190", res["knee"]["note"])
        else:
            expected = None if det["index"] is None else stdout["points"][det["index"]]["load"]
            self.assertEqual(res["knee"]["source"], "knee_detection")
        self.assertEqual(res["knee"]["load"], expected)
        if expected is None:
            self.assertIsNone(res["kv_cache_util_at_knee"])
        else:
            self.assertAlmostEqual(res["kv_cache_util_at_knee"], 0.25)

    def test_sweep5_with_knee_detection_index(self):
        rows, stdout = load_fixture("sweep5")
        stdout = copy.deepcopy(stdout)
        stdout["knee_detection"] = {"index": 3, "reason": None, "points": 5, "min_points": 5}
        res = analyze.analyze(rows, stdout)
        self.assertEqual(res["knee"]["load"], stdout["points"][3]["load"])
        self.assertEqual(res["knee"]["source"], "knee_detection")
        self.assertAlmostEqual(res["kv_cache_util_at_knee"], 0.25)
        self.assertIsNone(res["kv_cache_util_at_knee_reason"])

    def test_sweep5_flat_curve_is_null_with_reason(self):
        rows, stdout = load_fixture("sweep5")
        stdout = copy.deepcopy(stdout)
        stdout["knee"] = None
        stdout["knee_detection"] = {"index": None, "reason": "flat_curve",
                                    "points": 5, "min_points": 5}
        res = analyze.analyze(rows, stdout)
        self.assertIsNone(res["kv_cache_util_at_knee"])
        self.assertEqual(res["kv_cache_util_at_knee_reason"], "flat_curve")

    def test_sweep3_no_knee_either_shape(self):
        rows, stdout = load_fixture("sweep3")
        self.assertLess(len(stdout["points"]), analyze.KNEE_MIN_POINTS)
        shapes = [stdout]
        if "knee_detection" in stdout:
            legacy = copy.deepcopy(stdout)
            del legacy["knee_detection"]
            shapes.append(legacy)
        else:
            new = copy.deepcopy(stdout)
            new["knee"] = None
            new["knee_detection"] = {"index": None, "reason": "insufficient_points",
                                     "points": 3, "min_points": 5}
            shapes.append(new)
        for shape in shapes:
            res = analyze.analyze(rows, shape)
            self.assertIsNone(res["kv_cache_util_at_knee"])
            self.assertEqual(res["kv_cache_util_at_knee_reason"], "insufficient_points")
            self.assertTrue(res["knee"]["note"])

    def test_cli_text_and_json(self):
        nd = os.path.join(FIXTURES, "sweep3.ndjson")
        js = os.path.join(FIXTURES, "sweep3.stdout.json")
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            self.assertEqual(analyze.main(["analyze.py", nd, js]), 0)
        text = buf.getvalue()
        self.assertIn("kv_cache_util_at_knee\tnull\n", text)
        self.assertIn("kv_cache_util_at_knee_reason\tinsufficient_points\n", text)
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            self.assertEqual(analyze.main(["analyze.py", nd, js, "--json"]), 0)
        self.assertIsNone(json.loads(buf.getvalue())["kv_cache_util_at_knee"])


if __name__ == "__main__":
    unittest.main()
