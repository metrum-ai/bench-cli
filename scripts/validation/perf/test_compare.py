#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Offline unit tests for compare.py (issue #333): tolerance verdicts, the
# per-point noise floor, missing points, and the study-reference columns.
# Point roots are synthesized on the fly (deterministic values, no network,
# no GPU): every scalar is exact by construction.
#
# Usage:
#   python3 -m unittest -v test_compare.py

import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
COMPARE = HERE / "compare.py"

# Synthetic point shape: 8 measured requests, summary window 100 s, each
# request 3 completion tokens with ttft 0.05 s. tok_s is exact:
# tokens / window; tpot_p50_ms is exact per request.
REQUESTS_PER_POINT = 8
WINDOW_S = 100.0
TTFT_S = 0.05
TOKENS_PER_REQUEST = 3


def write_point(root, workload, concurrency, pass_, tok_s=None, tpot_ms=10.0,
                run_id="run-1"):
    """Write one point directory with exact-by-construction scalars.

    system_output_tok_s = sum(completion_tokens)/window = REQUESTS_PER_POINT
    * TOKENS_PER_REQUEST / WINDOW_S when tok_s is None. With tok_s given,
    window is scaled so the scalar lands exactly on tok_s.
    tpot per request = (latency - ttft)/(tokens-1) = tpot_ms/1000 exactly.
    """
    out_tokens = REQUESTS_PER_POINT * TOKENS_PER_REQUEST
    window = (out_tokens / tok_s) if tok_s is not None else WINDOW_S
    d = pathlib.Path(root) / str(pass_) / workload / f"c{concurrency}"
    d.mkdir(parents=True, exist_ok=True)
    rows = []
    for seq in range(REQUESTS_PER_POINT):
        latency = TTFT_S + (TOKENS_PER_REQUEST - 1) * (tpot_ms / 1000.0)
        rows.append({
            "schema_version": "metrum-ai-bench-cli.request.v3",
            "seq": seq, "run_id": run_id, "phase": "measure",
            "started_at": "2026-10-26T12:00:00Z",
            "completed_at": "2026-10-26T12:01:40Z",
            "error": None, "prompt_tokens": 100,
            "completion_tokens": TOKENS_PER_REQUEST,
            "ttft_s": TTFT_S, "latency_s": latency,
        })
    rows.append({
        "schema_version": "metrum-ai-bench-cli.summary.v3",
        "run_id": run_id, "partial": False,
        "config": {"run_id": run_id},
        "window_seconds": window,
    })
    (d / "requests.jsonl").write_text(
        "".join(json.dumps(r) + "\n" for r in rows))
    (d / "point.json").write_text(json.dumps(
        {"tool": "synthetic", "workload": workload, "concurrency": concurrency,
         "pass": str(pass_)}))
    return d


def write_grid(root, tok_s_fn, tpot_ms_fn, pass_=1):
    for w in ("W1", "W2"):
        for c in (1, 8, 32):
            write_point(root, w, c, pass_, tok_s=tok_s_fn(w, c),
                        tpot_ms=tpot_ms_fn(w, c))


def run_compare(baseline, candidate, *extra):
    """Run compare.py as a subprocess; return (rc, stdout+stderr, json-out)."""
    with tempfile.TemporaryDirectory() as td:
        out_json = pathlib.Path(td) / "table.json"
        proc = subprocess.run(
            [sys.executable, str(COMPARE),
             "--baseline-root", str(baseline),
             "--candidate-root", str(candidate),
             "--model", "q38-27b", "--pass", "1",
             "--out-json", str(out_json), *extra],
            capture_output=True, text=True)
        data = json.loads(out_json.read_text()) if out_json.exists() else None
        return proc.returncode, proc.stdout + proc.stderr, data


class ComparePower(unittest.TestCase):
    def test_nvidia_smi_power_header_and_units(self):
        with tempfile.TemporaryDirectory() as b, tempfile.TemporaryDirectory() as k:
            write_grid(b, lambda w, c: 100.0, lambda w, c: 40.0)
            write_grid(k, lambda w, c: 100.0, lambda w, c: 40.0)
            path = pathlib.Path(b) / "1" / "W1" / "c1" / "nvidia.csv"
            path.write_text("timestamp, power.draw [W]\nnow, 100.00 W\nlater, 200.00 W\n")
            # csv.DictReader preserves the leading space after the delimiter.
            # Actual nvidia-smi emits the same spacing.
            rc, out, data = run_compare(b, k)
            self.assertEqual(rc, 0, out)
            self.assertEqual(data["points"][0]["baseline"]["power_mean_w"], 150.0)


class CompareToleranceVerdicts(unittest.TestCase):
    def test_within_tolerance_passes(self):
        with tempfile.TemporaryDirectory() as b, tempfile.TemporaryDirectory() as k:
            write_grid(b, lambda w, c: 100.0, lambda w, c: 40.0)
            # +2.0% tok/s, -1.0% tpot: both inside the 5% gate.
            write_grid(k, lambda w, c: 102.0, lambda w, c: 39.6)
            rc, out, data = run_compare(b, k)
            self.assertEqual(rc, 0, out)
            self.assertEqual(data["verdict"], "PASS")
            for row in data["points"]:
                for metric in ("system_output_tok_s", "tpot_p50_ms"):
                    self.assertTrue(row["checks"][metric]["pass"], row)

    def test_beyond_tolerance_fails(self):
        with tempfile.TemporaryDirectory() as b, tempfile.TemporaryDirectory() as k:
            write_grid(b, lambda w, c: 100.0, lambda w, c: 40.0)
            # +6.0% tok/s breaches the 5% gate on every point.
            write_grid(k, lambda w, c: 106.0, lambda w, c: 40.0)
            rc, out, data = run_compare(b, k)
            self.assertEqual(rc, 1)
            self.assertEqual(data["verdict"], "FAIL")
            for row in data["points"]:
                self.assertFalse(row["checks"]["system_output_tok_s"]["pass"])
                self.assertTrue(row["checks"]["tpot_p50_ms"]["pass"])

    def test_missing_metric_scalar_fails_the_point(self):
        # usage_missing on every request -> no token scalars -> check fails.
        with tempfile.TemporaryDirectory() as b, tempfile.TemporaryDirectory() as k:
            write_grid(b, lambda w, c: 100.0, lambda w, c: 40.0)
            write_grid(k, lambda w, c: 100.0, lambda w, c: 40.0)
            d = pathlib.Path(k) / "1" / "W1" / "c1" / "requests.jsonl"
            rows = [json.loads(l) for l in d.read_text().splitlines()]
            for r in rows:
                if ".request." in r["schema_version"]:
                    r["usage_missing"] = True
            d.write_text("".join(json.dumps(r) + "\n" for r in rows))
            rc, _, data = run_compare(b, k)
            self.assertEqual(rc, 1)
            row = [r for r in data["points"]
                   if (r["workload"], r["concurrency"]) == ("W1", 1)][0]
            self.assertFalse(row["checks"]["system_output_tok_s"]["pass"])


class CompareNoiseFloor(unittest.TestCase):
    def test_noise_floor_widens_tolerance(self):
        # Candidate is +5.8% on tok/s: fails the 5% gate, passes with a
        # 6.5% floor on that metric for every point.
        with tempfile.TemporaryDirectory() as b, tempfile.TemporaryDirectory() as k:
            write_grid(b, lambda w, c: 100.0, lambda w, c: 40.0)
            write_grid(k, lambda w, c: 105.8, lambda w, c: 40.0)
            with tempfile.TemporaryDirectory() as td:
                floor = pathlib.Path(td) / "noise.json"
                floor.write_text(json.dumps({"q38-27b": {
                    w: {f"c{c}": {"system_output_tok_s": 6.5,
                                  "tpot_p50_ms": 1.0}
                         for c in (1, 8, 32)} for w in ("W1", "W2")}}))
                rc, out, data = run_compare(b, k, "--noise-floor", str(floor))
                self.assertEqual(rc, 0, out)
                self.assertEqual(data["verdict"], "PASS")
                for row in data["points"]:
                    # The effective gate for tok/s is 6.5, not 5.
                    self.assertEqual(
                        row["checks"]["system_output_tok_s"]["tolerance_pct"], 6.5)
                    self.assertTrue(row["checks"]["system_output_tok_s"]["pass"])
                    self.assertEqual(
                        row["checks"]["tpot_p50_ms"]["tolerance_pct"], 5.0)
                self.assertIn("noise floor up to 6.5%", out)

    def test_smaller_floor_does_not_narrow_the_gate(self):
        with tempfile.TemporaryDirectory() as b, tempfile.TemporaryDirectory() as k:
            write_grid(b, lambda w, c: 100.0, lambda w, c: 40.0)
            write_grid(k, lambda w, c: 105.8, lambda w, c: 40.0)
            with tempfile.TemporaryDirectory() as td:
                floor = pathlib.Path(td) / "noise.json"
                floor.write_text(json.dumps({"q38-27b": {
                    w: {f"c{c}": {"system_output_tok_s": 1.0}
                         for c in (1, 8, 32)} for w in ("W1", "W2")}}))
                rc, _, data = run_compare(b, k, "--noise-floor", str(floor))
                self.assertEqual(rc, 1)
                self.assertEqual(data["verdict"], "FAIL")

    def test_bad_noise_floor_key_is_an_error(self):
        with tempfile.TemporaryDirectory() as b, tempfile.TemporaryDirectory() as k:
            write_grid(b, lambda w, c: 100.0, lambda w, c: 40.0)
            write_grid(k, lambda w, c: 100.0, lambda w, c: 40.0)
            with tempfile.TemporaryDirectory() as td:
                floor = pathlib.Path(td) / "noise.json"
                floor.write_text(json.dumps({"q38-27b": {
                    "W1": {"banana": {"system_output_tok_s": 6.5}}}}))
                rc, out, _ = run_compare(b, k, "--noise-floor", str(floor))
                self.assertNotEqual(rc, 0)
                self.assertIn("bad concurrency key", out)


class CompareMissingPoints(unittest.TestCase):
    def test_missing_candidate_point_fails(self):
        with tempfile.TemporaryDirectory() as b, tempfile.TemporaryDirectory() as k:
            write_grid(b, lambda w, c: 100.0, lambda w, c: 40.0)
            write_grid(k, lambda w, c: 100.0, lambda w, c: 40.0)
            import shutil
            shutil.rmtree(pathlib.Path(k) / "1" / "W2" / "c32")
            rc, out, data = run_compare(b, k)
            self.assertEqual(rc, 1)
            self.assertEqual(data["verdict"], "FAIL")
            row = [r for r in data["points"]
                   if (r["workload"], r["concurrency"]) == ("W2", 32)][0]
            self.assertEqual(row["missing"], ["candidate"])
            self.assertIn("Missing points (1)", out)

    def test_missing_baseline_point_fails(self):
        with tempfile.TemporaryDirectory() as b, tempfile.TemporaryDirectory() as k:
            write_grid(b, lambda w, c: 100.0, lambda w, c: 40.0)
            write_grid(k, lambda w, c: 100.0, lambda w, c: 40.0)
            import shutil
            shutil.rmtree(pathlib.Path(b) / "1" / "W1" / "c8")
            rc, _, data = run_compare(b, k)
            self.assertEqual(rc, 1)
            row = [r for r in data["points"]
                   if (r["workload"], r["concurrency"]) == ("W1", 8)][0]
            self.assertEqual(row["missing"], ["baseline"])


class CompareStudyReference(unittest.TestCase):
    def test_study_deltas_are_present_and_informational(self):
        with tempfile.TemporaryDirectory() as b, tempfile.TemporaryDirectory() as k:
            write_grid(b, lambda w, c: 100.0, lambda w, c: 40.0)
            write_grid(k, lambda w, c: 102.0, lambda w, c: 40.0)
            with tempfile.TemporaryDirectory() as td:
                ref = pathlib.Path(td) / "study.json"
                ref.write_text(json.dumps([
                    {"workload": w, "concurrency": c,
                     "system_output_tok_s": 100.0, "tpot_p50_ms": 40.0,
                     "ttft_p50_ms": 50.0, "requests_s": 0.08,
                     "power_mean_w": 300.0}
                    for w in ("W1", "W2") for c in (1, 8, 32)]))
                rc, out, data = run_compare(b, k, "--reference", str(ref))
                self.assertEqual(rc, 0, out)
                for row in data["points"]:
                    study = row["study"]["system_output_tok_s"]
                    self.assertAlmostEqual(study["baseline_delta_pct"], 0.0)
                    self.assertAlmostEqual(study["candidate_delta_pct"], 2.0)


if __name__ == "__main__":
    unittest.main()
