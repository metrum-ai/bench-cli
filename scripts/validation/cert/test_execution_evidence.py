#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
import execution_evidence as evidence
import result_writer


class ExecutionEvidenceTests(unittest.TestCase):
    def test_floor(self):
        with self.assertRaisesRegex(RuntimeError, "Python 3.12"):
            evidence.require_python((3, 11))
        evidence.require_python((3, 12))

    def test_atomic_record_hashes_and_report(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "capture.txt").write_text("captured")
            record = {"ID": "BC-GPU-001", "result": "FAIL", "candidate_sha": "a" * 40,
                      "observed_values": {"request_count": None}, "evidence_paths": ["capture.txt"]}
            with patch.dict("os.environ", {}, clear=True):
                path = result_writer.write_record(root, record)
            saved = json.loads(path.read_text())
            self.assertEqual(saved["output_hashes"]["capture.txt"], evidence.digest(root / "capture.txt"))
            self.assertIsNone(saved["observed_values"]["request_count"])
            self.assertFalse(saved["carry_over_eligibility"]["eligible"])
            self.assertEqual(saved["infra_history"][0]["type"], "failed_or_interrupted")
            self.assertEqual(evidence.report_rows(root), [saved])
            with self.assertRaises(FileExistsError):
                result_writer.write_record(root, record)
            (root / "capture.txt").write_text("modified")
            with self.assertRaisesRegex(ValueError, "hash mismatch"):
                evidence.report_rows(root)

    def test_phase_append(self):
        with tempfile.TemporaryDirectory() as temporary:
            for phase in evidence.PHASES:
                evidence.phase(temporary, "BC-GPU-001", phase)
            lines = (Path(temporary) / "phases.jsonl").read_text().splitlines()
            self.assertEqual([json.loads(line)["phase"] for line in lines], list(evidence.PHASES))
            with self.assertRaises(ValueError):
                evidence.phase(temporary, "../bad", "rent")
            with self.assertRaises(ValueError):
                evidence.phase(temporary, "BC-GPU-001", "invented")


if __name__ == "__main__":
    unittest.main()
