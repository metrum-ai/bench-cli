#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Mocked runner dispatch/exit regressions; never certification evidence."""
import argparse
import io
import os
from contextlib import ExitStack, redirect_stderr
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import g3_runner
import perf_p1_runner
import r_runner


class RunnerSelectionTests(unittest.TestCase):
    def check_unknown(self, runner, values):
        for target in values:
            with self.subTest(target=target), patch.dict(os.environ, {"CERT_CASE_ID": target}), \
                    patch.object(argparse.ArgumentParser, "parse_args", side_effect=AssertionError("invalid case reached parsing")) as parse, \
                    patch.object(Path, "read_text") as read, \
                    patch.object(runner.subprocess, "run") as run, \
                    redirect_stderr(io.StringIO()) as stderr:
                with self.assertRaises(SystemExit) as raised:
                    runner.main()
                self.assertEqual(raised.exception.code, 2)
                self.assertIn("unknown CERT_CASE_ID", stderr.getvalue())
                self.assertIn(repr(target), stderr.getvalue())
                self.assertIn(runner.__name__ + ".py", stderr.getvalue())
                parse.assert_not_called()
                read.assert_not_called()
                run.assert_not_called()

    def test_p1_unknown_case_fails_before_parsing_or_work(self):
        self.check_unknown(perf_p1_runner, ("BC-UNKNOWN-999", "BC-PERF-002", "BC-HW-001", ""))

    def test_r_unknown_case_fails_before_parsing_or_work(self):
        self.check_unknown(r_runner, ("BC-UNKNOWN-999", "BC-PERF-001", "BC-REASON-005", ""))

    def test_known_selections_and_unset_bulk_mode_reach_argument_parsing(self):
        choices = ((perf_p1_runner, (None, "BC-PERF-001", "BC-TELEM-001")),
                   (r_runner, (None, "BC-REASON-001", "BC-REASON-002", "BC-REASON-003",
                               "BC-REASON-004", "BC-HW-001")))
        for runner, cases in choices:
            for target in cases:
                env = {} if target is None else {"CERT_CASE_ID": target}
                with self.subTest(runner=runner.__name__, target=target), \
                        patch.dict(os.environ, env, clear=True), \
                        patch.object(argparse.ArgumentParser, "parse_args", side_effect=RuntimeError("parsing reached")):
                    with self.assertRaisesRegex(RuntimeError, "parsing reached"):
                        runner.main()

    def run_g3(self, target, failed_order=None):
        with tempfile.TemporaryDirectory() as directory, ExitStack() as stack:
            root = Path(directory)
            (root / "evidence").mkdir()
            (root / "evidence/gpu.csv").write_text("fixture-class,fixture-driver\n")
            args = SimpleNamespace(out=root, candidate=root / "candidate", baseline=root / "baseline",
                                   manifest=root / "manifest", sha="a" * 40, model="fixture-model",
                                   revision="fixture-revision", image="fixture-image", owner="fixture",
                                   start_utc="2026-01-01T00:00:00Z", evidence_link="fixture")
            env = {} if target is None else {"CERT_CASE_ID": target}
            stack.enter_context(patch.dict(os.environ, env, clear=True))
            stack.enter_context(patch.object(argparse.ArgumentParser, "parse_args", return_value=args))
            stack.enter_context(patch.object(g3_runner, "run_cell", return_value=({}, {}, 0)))
            stack.enter_context(patch.object(g3_runner.manifest_tool, "sha256_file", return_value="b" * 64))
            stack.enter_context(patch.object(g3_runner.g3_checks, "verify_complete_control", return_value={"checked": True}))
            def window(_data, _trace, order):
                if order == failed_order:
                    raise ValueError("synthetic ordering failure")
                return {"checked": True}
            stack.enter_context(patch.object(g3_runner.g3_checks, "verify_window", side_effect=window))
            stack.enter_context(patch.object(g3_runner.g3_checks, "verify_goodput_median", return_value={"checked": True}))
            writer = stack.enter_context(patch.object(g3_runner.result_writer, "write_record"))
            code = g3_runner.main()
            return code, [call.args[1] for call in writer.call_args_list]

    def test_g3_failing_008_with_present_truthy_cell_dicts_returns_one(self):
        code, records = self.run_g3("BC-GPU-008", "error-first")
        self.assertEqual(code, 1)
        self.assertEqual([(r["ID"], r["result"]) for r in records], [("BC-GPU-008", "FAIL")])
        cells = records[0]["observed_values"]["cells"]
        self.assertTrue(all(isinstance(cells[c], dict) and cells[c] for c in ("control", "error-first", "error-last")))
        self.assertIs(cells["BC-GPU-008"], False)

    def test_g3_bulk_checks_both_case_booleans(self):
        code, records = self.run_g3(None)
        self.assertEqual(code, 0)
        self.assertEqual([(r["ID"], r["result"]) for r in records], [("BC-GPU-008", "PASS"), ("BC-GPU-009", "PASS")])

    def test_g3_selected_009_checks_only_its_case_boolean(self):
        code, records = self.run_g3("BC-GPU-009", "error-first")
        self.assertEqual(code, 0)
        self.assertEqual([(r["ID"], r["result"]) for r in records], [("BC-GPU-009", "PASS")])

    def test_g3_no_selected_case_does_not_pass_on_cell_dicts(self):
        code, records = self.run_g3("BC-UNKNOWN-999")
        self.assertEqual(code, 1)
        self.assertEqual(records, [])


if __name__ == "__main__":
    unittest.main()
