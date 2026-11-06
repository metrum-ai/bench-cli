#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
import copy
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
import case_launcher
from execution_evidence import digest


class CaseLauncherTests(unittest.TestCase):
    def test_pins_and_preflight_fields(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "binary"
            binary.write_text("#!/bin/sh\nexit 0\n")
            binary.chmod(0o755)
            sut = root / "sut.json"
            sut.write_text(json.dumps({"host_os": "Linux", "memory_gb": 128, "gpu": {"memory_gb": 48}}))
            def pin(path):
                return {"path": str(path), "sha256": digest(path)}
            manifest = {"candidate_sha": "a" * 40, "candidate": pin(binary), "sut": pin(sut),
                        "all_smi": pin(binary), "tokenizer": {**pin(binary), "revision": "b" * 40},
                        "cases": {"BC-GPU-001": {"launcher": pin(binary), "out": str(root / "attempt"),
                                                    "command": [str(binary), "--out", str(root / "attempt")]}}}
            with patch.object(case_launcher.subprocess, "run") as run:
                self.assertEqual(case_launcher.preflight(manifest), ["BC-GPU-001"])
                run.assert_called_once()
            for field in ("host_os", "memory_gb"):
                bad = {"host_os": "Linux", "memory_gb": 128, "gpu": {"memory_gb": 48}}
                bad.pop(field)
                sut.write_text(json.dumps(bad))
                invalid = copy.deepcopy(manifest)
                invalid["sut"] = pin(sut)
                with self.assertRaisesRegex(ValueError, "SUT"):
                    case_launcher.preflight(invalid)
            with self.assertRaisesRegex(ValueError, "hash mismatch"):
                case_launcher.preflight(manifest)
            sut.write_text(json.dumps({"host_os": "Linux", "memory_gb": 128, "gpu": {"memory_gb": 48}}))
            for field in ("all_smi", "tokenizer", "candidate"):
                invalid = copy.deepcopy(manifest)
                invalid[field]["sha256"] = "0" * 64
                with self.assertRaisesRegex(ValueError, "hash mismatch"):
                    case_launcher.preflight(invalid)
            invalid = copy.deepcopy(manifest)
            invalid["tokenizer"]["revision"] = "main"
            with self.assertRaisesRegex(ValueError, "revision"):
                case_launcher.preflight(invalid)


    def test_failed_launch_writes_typed_record(self):
        import subprocess
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            launcher = root / "launcher"
            launcher.write_text("#!/bin/sh\nexit 143\n")
            launcher.chmod(0o755)
            def pin():
                return {"path": str(launcher), "sha256": digest(launcher)}
            out = root / "attempt"
            manifest = {"candidate_sha": "a" * 40, "candidate": pin(), "sut": pin(), "all_smi": pin(),
                        "tokenizer": pin(), "cases": {"BC-GPU-001": {"out": str(out), "launcher": pin(),
                        "command": [str(launcher), "--out", str(out)]}}}
            frozen = root / "frozen.json"
            frozen.write_text(json.dumps(manifest))
            with self.assertRaises(subprocess.CalledProcessError):
                case_launcher.run_case(frozen, manifest, "BC-GPU-001")
            record = json.loads((out / "results/BC-GPU-001.json").read_text())
            self.assertEqual(record["result"], "FAIL")
            self.assertIsNone(record["observed_values"]["request_count"])
            self.assertEqual(record["infra_history"][0]["type"], "infra_interrupted")
            self.assertEqual(record["input_hashes"]["execution_manifest"], digest(frozen))
            with self.assertRaises(FileExistsError):
                case_launcher.run_case(frozen, manifest, "BC-GPU-001")


if __name__ == "__main__":
    unittest.main()
