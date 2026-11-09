#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
import csv
import os
import tempfile
import unittest
from contextlib import ExitStack
from pathlib import Path
from unittest.mock import patch
import r_runner


class IndividualReasoningTests(unittest.TestCase):
    def run_selected(self, case, gpu="NVIDIA H100 80GB HBM3", failed_prerequisite=False):
        with tempfile.TemporaryDirectory() as temporary, ExitStack() as stack:
            root = Path(temporary)
            evidence = root / "evidence"
            evidence.mkdir()
            (evidence / "gpu.csv").write_text(f"{gpu},fixture-driver\n")
            binary = root / "binary"
            binary.write_text("fixture")
            argv = ["r_runner", "--out", str(root), "--candidate", str(binary), "--baseline", str(binary),
                    "--manifest", str(binary), "--prompt", str(binary), "--sha", "a" * 40, "--model", "fixture/model",
                    "--revision", "b" * 40, "--image", "fixture-image", "--start-utc", "2026-01-01T00:00:00Z",
                    "--owner", "fixture", "--evidence-link", "fixture", "--port", "8000", "--proxy-port", "8001"]
            stack.enter_context(patch("sys.argv", argv))
            stack.enter_context(patch.dict(os.environ, {"CERT_CASE_ID": case}))
            def fake_cell(args, label, *rest):
                return "", "", 0, [f"evidence/{label}-requests.jsonl"]
            calls = stack.enter_context(patch.object(r_runner, "run_cell", side_effect=fake_cell))
            write = stack.enter_context(patch.object(r_runner.result_writer, "write_record"))
            for name in ("truncation_cell", "adequate_cell", "safety_cell", "control_cell", "thinking_live_cell"):
                kwargs = {"side_effect": ValueError("synthetic prerequisite failure")} if name == "truncation_cell" and failed_prerequisite else {"return_value": {"checked": True}}
                stack.enter_context(patch.object(r_runner.r_checks, name, **kwargs))
            stack.enter_context(patch.object(r_runner, "baseline_observed", return_value={"checked": True}))
            code = r_runner.main()
            records = [call.args[1] for call in write.call_args_list]
            return code, records, calls.call_count

    def test_control_does_not_run_sibling_workloads(self):
        code, records, count = self.run_selected("BC-REASON-003")
        self.assertEqual(code, 0)
        self.assertEqual([r["ID"] for r in records], ["BC-REASON-003"])
        self.assertEqual(count, 2)  # candidate and baseline control only

    def test_hardware_keeps_three_required_reasoning_prerequisites(self):
        code, records, count = self.run_selected("BC-HW-001")
        self.assertEqual(code, 0)
        self.assertEqual([r["ID"] for r in records], ["BC-HW-001"])
        self.assertEqual(records[0]["result"], "PASS")
        self.assertEqual(records[0]["observed_values"]["reasoning_cases_passed"], 3)
        self.assertEqual(records[0]["thresholds"]["gpu_class"], "NVIDIA H100")
        self.assertEqual(count, 11)  # six truncation + three adequate + two control cells
        paths = records[0]['evidence_paths']
        self.assertEqual(len(paths), len(set(paths)))
        self.assertIn('evidence/r-128-default-requests.jsonl', paths)
        self.assertIn('evidence/r-4096-candidate-requests.jsonl', paths)
        self.assertIn('evidence/r-control-candidate-requests.jsonl', paths)

    def test_hardware_fails_for_wrong_class_or_failed_prerequisite(self):
        for gpu, failed in (("NVIDIA L40S", False), ("NVIDIA H1000", False),
                            ("NVIDIA RTX PRO 6000 Blackwell Server Edition", False),
                            ("NVIDIA H100 80GB HBM3", True)):
            with self.subTest(gpu=gpu, failed=failed):
                code, records, _ = self.run_selected("BC-HW-001", gpu, failed)
                self.assertEqual(code, 1)
                self.assertEqual(records[0]["result"], "FAIL")
                self.assertIn("requires NVIDIA H100", records[0]["observed_values"]["error"])

    def test_hardware_gate_matches_catalog(self):
        catalog = Path(__file__).resolve().parents[3] / "docs/RELEASE_CERTIFICATION_CATALOG.csv"
        with catalog.open(newline="", encoding="utf-8") as stream:
            row = next(row for row in csv.DictReader(stream) if row["ID"] == "BC-HW-001")
        self.assertEqual(row["Hardware"], "NVIDIA H100")


if __name__ == "__main__":
    unittest.main()
