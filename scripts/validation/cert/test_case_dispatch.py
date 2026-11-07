#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
import ast
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
import case_dispatch


class CaseDispatchTests(unittest.TestCase):
    def test_mapping_matches_real_case_id_literals(self):
        root = Path(__file__).parent
        for name, cases in case_dispatch.RUNNERS.items():
            if name in ("perf_all_smi.py", "live_modality_runner.py"):
                continue  # collector/helper with caller-supplied case identities
            # W collection deliberately cannot publish an externally signed result.
            source = "w_finalize.py" if name == "w_runner.py" else name
            values = {node.value for node in ast.walk(ast.parse((root / source).read_text()))
                      if isinstance(node, ast.Constant) and isinstance(node.value, str)}
            self.assertTrue(cases <= values, (name, cases - values))

    def test_selected_runner_only(self):
        self.assertTrue(case_dispatch.run_selected(["python3", "/fixture/g1_clock_runner.py"], "BC-GPU-001"))
        self.assertFalse(case_dispatch.run_selected(["python3", "/fixture/g1_variants_runner.py"], "BC-GPU-001"))
        with self.assertRaises(ValueError):
            case_dispatch.run_selected(["python3", "unknown.py"], "BC-GPU-001")
        with patch.dict(os.environ, {"CERT_CASE_ID": "BC-GPU-008"}):
            self.assertTrue(case_dispatch.selected("BC-GPU-008"))
            self.assertFalse(case_dispatch.selected("BC-GPU-009"))

    def test_shared_shell_skips_sibling_before_execution(self):
        common = Path(__file__).with_name("common.sh").resolve()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for name in ("g1_clock_runner.py", "g1_variants_runner.py"):
                (root / name).write_text("# Copyright (c) 2026 Metrum AI, Inc.\nprint('" + name + "')\n")
            command = 'source "$1"; export CERT_CASE_ID=BC-GPU-001; cert_run python3 "$2/g1_variants_runner.py"; cert_run python3 "$2/g1_clock_runner.py"'
            result = subprocess.run(["bash", "-c", command, "test", str(common), str(root)], capture_output=True, text=True, check=True)
            self.assertEqual(result.stdout.strip(), "g1_clock_runner.py")


if __name__ == "__main__":
    unittest.main()
