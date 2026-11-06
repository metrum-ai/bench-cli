#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
import os
import tempfile
import unittest
from contextlib import ExitStack
from pathlib import Path
from unittest.mock import patch
import r_runner


class IndividualReasoningTests(unittest.TestCase):
    def run_selected(self, case):
        with tempfile.TemporaryDirectory() as temporary, ExitStack() as stack:
            root = Path(temporary)
            evidence = root / "evidence"
            evidence.mkdir()
            (evidence / "gpu.csv").write_text("L40S,fixture-driver\n")
            binary = root / "binary"
            binary.write_text("fixture")
            argv = ["r_runner", "--out", str(root), "--candidate", str(binary), "--baseline", str(binary),
                    "--manifest", str(binary), "--prompt", str(binary), "--sha", "a" * 40, "--model", "fixture/model",
                    "--revision", "b" * 40, "--image", "fixture-image", "--start-utc", "2026-01-01T00:00:00Z",
                    "--owner", "fixture", "--evidence-link", "fixture", "--port", "8000", "--proxy-port", "8001"]
            stack.enter_context(patch("sys.argv", argv))
            stack.enter_context(patch.dict(os.environ, {"CERT_CASE_ID": case}))
            calls = stack.enter_context(patch.object(r_runner, "run_cell", return_value=("", "", 0, [])))
            write = stack.enter_context(patch.object(r_runner.result_writer, "write_record"))
            for name in ("truncation_cell", "adequate_cell", "safety_cell", "control_cell", "thinking_live_cell"):
                stack.enter_context(patch.object(r_runner.r_checks, name, return_value={"checked": True}))
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
        self.assertEqual(records[0]["observed_values"]["reasoning_cases_passed"], 3)
        self.assertEqual(count, 11)  # six truncation + three adequate + two control cells


if __name__ == "__main__":
    unittest.main()
