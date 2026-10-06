#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Tests for analyze.py. Run: python3 -m unittest discover -s docs/queries

fixtures/sweep5, sweep5_no_bend and sweep3 are recorded runs of
metrum-ai-bench-cli-strategic against metrum-ai-bench-cli-mock-server
--telemetry-fixture (fixtures/record.sh). sweep5 goes through
fixtures/capacity_proxy.py (4 requests at a time), so p95 bends past c=4 and
the recorded knee_detection has a real knee (#232, #240); sweep5_no_bend is
the same sweep straight to the fixed-latency mock. Legacy-shape tests delete
`knee_detection` to exercise the `knee` fallback for builds before #190.
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


RECORDED = ("sweep5", "sweep5_no_bend", "sweep3")


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
        hq = analyze.histogram_quantile
        self.assertAlmostEqual(hq(b, 0.5)["value"], 0.1 + 0.4 * 10 / 20)
        self.assertAlmostEqual(hq(b, 0.95)["value"], 0.5 + 0.5 * 8 / 10)
        self.assertIsNone(hq(b, 0.5)["reason"])
        self.assertIsNone(hq(b, 0.5)["bound"])

    def test_histogram_quantile_all_mass_below_first_bound(self):
        # The #231 live case: every request_prefill_time value is <= 0.3 s.
        # Interpolating from 0 used to report p50/p95 = 0.15/0.285.
        b = [(0.3, 40), (0.5, 40), (1.0, 40), (float("inf"), 40)]
        for q in (0.5, 0.95):
            self.assertEqual(
                analyze.histogram_quantile(b, q),
                {"value": None, "reason": "below_first_bucket", "bound": 0.3},
            )

    def test_histogram_quantile_spanning_buckets(self):
        # 10 <= 0.1, 20 more <= 0.5, 10 above 1.0: one rank per region.
        b = [(0.1, 10), (0.5, 30), (1.0, 30), (float("inf"), 40)]
        hq = analyze.histogram_quantile
        self.assertEqual(hq(b, 0.25), {"value": None, "reason": "below_first_bucket", "bound": 0.1})
        self.assertAlmostEqual(hq(b, 0.5)["value"], 0.1 + 0.4 * 10 / 20)
        self.assertEqual(hq(b, 0.95), {"value": None, "reason": "above_last_bucket", "bound": 1.0})

    def test_histogram_quantile_all_mass_in_inf(self):
        b = [(0.3, 0), (1.0, 0), (float("inf"), 10)]
        for q in (0.5, 0.95):
            self.assertEqual(
                analyze.histogram_quantile(b, q),
                {"value": None, "reason": "above_last_bucket", "bound": 1.0},
            )

    def test_histogram_quantile_edges(self):
        empty = {"value": None, "reason": None, "bound": None}
        self.assertEqual(analyze.histogram_quantile([(1.0, 0), (float("inf"), 0)], 0.5), empty)
        self.assertEqual(analyze.histogram_quantile([], 0.5), empty)
        # Only +Inf: no finite bound to report.
        self.assertEqual(analyze.histogram_quantile([(float("inf"), 5)], 0.5), empty)

    def test_stage_histogram_uses_bucket_deltas(self):
        m = "vllm:time_to_first_token_seconds"
        rows = []
        for t, counts in ((0.0, (100, 100, 100)), (1.0, (100, 110, 120))):
            for le, c in zip(("0.1", "0.5", "+Inf"), counts):
                rows.append(tele(m + "_bucket", t, c, le=le, engine="0"))
        # Deltas: 0 <= 0.1, 10 <= 0.5, 20 total.
        self.assertAlmostEqual(
            analyze.stage_histogram_quantile(rows, m, 0.5)["value"], 0.1 + 0.4 * 10 / 10
        )
        p95 = analyze.stage_histogram_quantile(rows, m, 0.95)
        self.assertEqual((p95["reason"], p95["bound"]), ("above_last_bucket", 0.5))

    def test_stage_histogram_drops_label_set_with_reset(self):
        m = "vllm:e2e_request_latency_seconds"
        rows = []
        # engine 0 is clean; engine 1 resets its 0.5 bucket mid-window.
        for t, e0, e1 in ((0.0, (0, 10, 10), (50, 60, 60)), (1.0, (0, 10, 20), (55, 1, 70))):
            for engine, counts in (("0", e0), ("1", e1)):
                for le, c in zip(("0.1", "0.5", "+Inf"), counts):
                    rows.append(tele(m + "_bucket", t, c, le=le, engine=engine))
        # Only engine 0 counts: deltas 0, 0, 10, so every rank is in +Inf.
        p50 = analyze.stage_histogram_quantile(rows, m, 0.5)
        self.assertEqual((p50["value"], p50["reason"], p50["bound"]),
                         (None, "above_last_bucket", 0.5))

    def test_engine_histograms_text_and_json(self):
        m = "vllm:request_prefill_time_seconds"
        rows = [{"kind": "stage", "run_id": "r", "stage": "c1", "phase": "measure",
                 "load": 1, "t_start_ns": 0, "t_end_ns": int(2e9)}]
        for t, counts in ((0.5, (0, 0, 0)), (1.5, (40, 40, 40))):
            for le, c in zip(("0.3", "0.5", "+Inf"), counts):
                rows.append(tele(m + "_bucket", t, c, le=le, engine="0"))
        res = analyze.analyze(rows)
        h = res["stages"][0]["engine_histograms"][m]
        self.assertEqual(h, {"p50": None, "p50_reason": "below_first_bucket", "p50_bound": 0.3,
                             "p95": None, "p95_reason": "below_first_bucket", "p95_bound": 0.3})
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            analyze.render(res)
        self.assertIn(f"\t{m}\t<=0.3\t<=0.3\n", buf.getvalue())


class PowerEnergySources(unittest.TestCase):
    def test_one_power_and_energy_source_per_stage(self):
        rows = []
        for t, e in ((0.0, 0.0), (1.0, 300.0)):
            rows += [
                tele("all_smi_gpu_power_consumption_watts", t, 300.0),
                tele("DCGM_FI_DEV_POWER_USAGE", t, 300.0),
                tele("all_smi_chassis_power_consumption_watts", t, 900.0, unit="W"),
                tele("all_smi_gpu_energy_hw_millijoules_total", t, e * 1000.0),
                tele("DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION", t, e, unit="J"),
                tele("all_smi_energy_consumed_joules_total", t, e, unit="J"),
            ]
        for r in rows:
            r["unit"] = r["labels"].pop("unit", None)
        stage = {"kind": "stage", "run_id": "r", "stage": 1, "load": 1,
                 "phase": "measure", "t_start_ns": 0, "t_end_ns": int(2e9)}
        s = analyze.analyze(rows + [stage])["stages"][0]
        self.assertAlmostEqual(s["power_mean_w"], 300.0)
        self.assertAlmostEqual(s["energy_counter_j"], 300.0)
        self.assertAlmostEqual(s["energy_trap_j"], 300.0)
        self.assertEqual(s["sources"]["power"], "all_smi_gpu_power_consumption_watts")
        self.assertEqual(s["sources"]["energy_counter"],
                         "all_smi_gpu_energy_hw_millijoules_total")

    def test_dcgm_fallback_and_node_meters_excluded(self):
        rows = [
            {"metric": "DCGM_FI_DEV_POWER_USAGE", "unit": "W"},
            {"metric": "ipmi_power_watts", "unit": "W"},
            {"metric": "redfish_chassis_power_average_consumed_watts", "unit": "W"},
            {"metric": "node_power_watts", "unit": "W"},
        ]
        self.assertEqual(
            analyze.pick_metric(rows, analyze.POWER_PREFERENCE, analyze.is_power_draw),
            "DCGM_FI_DEV_POWER_USAGE")
        self.assertIsNone(
            analyze.pick_metric(rows[1:], analyze.POWER_PREFERENCE, analyze.is_power_draw))


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

    def test_sweep5_no_bend_has_at_least_five_points(self):
        rows, stdout = load_fixture("sweep5_no_bend")
        stages = [r for r in rows if r.get("kind") == "stage" and r.get("phase") == "measure"]
        self.assertGreaterEqual(len(stdout["points"]), analyze.KNEE_MIN_POINTS)
        self.assertEqual(len(stages), len(stdout["points"]))

    def test_recorded_stdout_has_knee_detection(self):
        loaded = [load_fixture(name)[1] for name in RECORDED]
        self.assertEqual(len(loaded), 3)
        for stdout in loaded:
            self.assertIn("knee_detection", stdout)
            self.assertEqual(stdout["knee_detection"]["min_points"], analyze.KNEE_MIN_POINTS)
        s5, nb, s3 = loaded
        # The capacity gate holds p95 flat to c=4, then it roughly doubles.
        det = s5["knee_detection"]
        self.assertEqual(det["index"], 2)
        self.assertIsNone(det["reason"])
        self.assertGreaterEqual(det["p95_rise"], det["min_p95_rise"])
        self.assertEqual(s5["points"][2]["load"], 4)
        det = nb["knee_detection"]
        self.assertIsNone(det["index"])
        self.assertEqual(det["reason"], "no_bend")
        self.assertLess(det["p95_rise"], det["min_p95_rise"])
        self.assertIsNone(nb["knee"])
        self.assertEqual(s3["knee_detection"]["index"], None)
        self.assertEqual(s3["knee_detection"]["reason"], "insufficient_points")
        self.assertIsNone(s3["knee"])

    def test_every_recorded_stage_spans_two_scrapes(self):
        # record.sh sizes stages so none falls between scrapes; a stage with
        # no samples would read power_mean_w and preemptions_delta as None.
        checked = 0
        for name in RECORDED:
            rows, _ = load_fixture(name)
            stages = [r for r in rows if r.get("kind") == "stage" and r.get("phase") == "measure"]
            self.assertTrue(stages, name)
            for st in stages:
                for metric in ("all_smi_gpu_power_consumption_watts",
                               "vllm:num_preemptions_total"):
                    n = sum(1 for r in rows if r.get("metric") == metric
                            and st["t_start_ns"] <= r["t_ns"] <= st["t_end_ns"])
                    self.assertGreaterEqual(n, 2, (name, st["load"], metric))
                    checked += 1
        self.assertEqual(checked, 2 * (5 + 5 + 3))

    def test_sweep5_power_is_one_gpu(self):
        # The mock serves the same 200..249 W on DCGM and all-smi; summing both
        # exporters would read 400+ W.
        checked = 0
        for name in ("sweep5", "sweep5_no_bend"):
            rows, stdout = load_fixture(name)
            for s in analyze.analyze(rows, stdout)["stages"]:
                self.assertGreaterEqual(s["power_mean_w"], 200.0)
                self.assertLess(s["power_mean_w"], 250.0)
                self.assertEqual(s["sources"]["power"], "all_smi_gpu_power_consumption_watts")
                checked += 1
        self.assertEqual(checked, 10)

    def test_knee_index_out_of_range(self):
        rows, stdout = load_fixture("sweep5")
        stdout = copy.deepcopy(stdout)
        stdout["knee_detection"] = {"index": 9, "reason": None, "points": 5, "min_points": 5}
        res = analyze.analyze(rows, stdout)
        self.assertIsNone(res["kv_cache_util_at_knee"])
        self.assertEqual(res["kv_cache_util_at_knee_reason"], "knee_index_out_of_range")

    def test_legacy_fallback_counts_points_with_p95(self):
        rows, stdout = load_fixture("sweep5")
        legacy = copy.deepcopy(stdout)
        del legacy["knee_detection"]
        legacy["knee"] = legacy["points"][2]
        res = analyze.analyze(rows, legacy)
        self.assertEqual(res["knee"]["source"], "legacy_knee")
        self.assertEqual(res["knee"]["load"], legacy["points"][2]["load"])
        legacy["points"][4]["p95_s"] = None
        res = analyze.analyze(rows, legacy)
        self.assertIsNone(res["kv_cache_util_at_knee"])
        self.assertEqual(res["kv_cache_util_at_knee_reason"], "insufficient_points")

    def test_sweep5_stage_metrics(self):
        for name in ("sweep5", "sweep5_no_bend"):
            rows, stdout = load_fixture(name)
            res = analyze.analyze(rows, stdout)
            self.assertEqual(len(res["stages"]), 5)
            final = max(r["value"] for r in rows
                        if r.get("metric") == "vllm:num_preemptions_total")
            self.assertGreater(final, 0)
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
        # The mock KV gauge is constant (0.25), so the kv-at-knee value alone
        # cannot tell stages apart; the knee load asserts pin the stage index.
        rows, stdout = load_fixture("sweep5")
        res = analyze.analyze(rows, stdout)
        self.assertEqual(res["knee"]["source"], "knee_detection")
        self.assertEqual(res["knee"]["load"], 4)
        self.assertAlmostEqual(res["kv_cache_util_at_knee"], 0.25)
        self.assertIsNone(res["kv_cache_util_at_knee_reason"])
        legacy = copy.deepcopy(stdout)
        del legacy["knee_detection"]
        res = analyze.analyze(rows, legacy)
        self.assertEqual(res["knee"]["source"], "legacy_knee")
        self.assertIn("predates #190", res["knee"]["note"])
        self.assertEqual(res["knee"]["load"], 4)
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

    def test_sweep5_no_bend_is_null_with_reason(self):
        # #232: recorded against the fixed-latency mock, p95 rose less than 20%.
        rows, stdout = load_fixture("sweep5_no_bend")
        self.assertEqual(stdout["knee_detection"]["reason"], "no_bend")
        res = analyze.analyze(rows, stdout)
        self.assertIsNone(res["kv_cache_util_at_knee"])
        self.assertEqual(res["kv_cache_util_at_knee_reason"], "no_bend")
        self.assertIsNone(res["knee"]["load"])

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
