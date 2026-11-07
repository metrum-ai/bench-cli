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
            self.assertEqual(saved["infra_history"][0]["type"], "case_failed")
            self.assertEqual(evidence.report_rows(root), [saved])
            with self.assertRaises(FileExistsError):
                result_writer.write_record(root, record)
            (root / "capture.txt").write_text("modified")
            with self.assertRaisesRegex(ValueError, "hash mismatch"):
                evidence.report_rows(root)

    def test_lane_pins_are_rechecked_and_missing_input_is_retained_on_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            prompt = root / "prompt.jsonl"
            prompt.write_text("original")
            manifest = root / "manifest.json"
            manifest.write_text(json.dumps({"lanes": {"G1": {"prompt": {"path": str(prompt), "sha256": evidence.digest(prompt)}},
                                                    "G2": {"unused": {"path": str(root / "not-local"), "sha256": "0" * 64}}}}))
            record = {"ID": "BC-GPU-001", "result": "FAIL", "candidate_sha": "a" * 40, "evidence_paths": []}
            with patch.dict("os.environ", {"CERT_MANIFEST": str(manifest), "CERT_LANE": "G1"}, clear=True):
                saved = evidence.enrich(root, record)
                self.assertEqual(saved["input_hashes"]["lane.G1.prompt"], evidence.digest(prompt))
                self.assertNotIn("input_integrity_errors", saved)
                prompt.write_text("changed")
                with self.assertRaisesRegex(ValueError, "input changed"):
                    evidence.enrich(root, {**record, "result": "PASS"})
                saved = evidence.enrich(root, record)
                self.assertTrue(saved["input_integrity_errors"])
                prompt.unlink()
                saved = evidence.enrich(root, record)
                self.assertIsNone(saved["input_hashes"]["lane.G1.prompt"])
                self.assertTrue(saved["input_integrity_errors"])

    def test_request_phases_use_observed_clocks(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            rows = [{"schema_version": "metrum-ai-bench-cli.request.v5",
                     "started_at": "2026-01-01T00:00:01Z", "completed_at": "2026-01-01T00:00:03Z"},
                    {"kind": "send", "utc": "2026-01-01T00:00:02Z"},
                    {"kind": "complete", "utc": "2026-01-01T00:00:04Z"}]
            (root / "observed.jsonl").write_text("\n".join(json.dumps(row) for row in rows) + "\n{interrupted")
            with patch.dict("os.environ", {}, clear=True):
                path = result_writer.write_record(root, {"ID": "BC-GPU-001", "result": "FAIL",
                    "candidate_sha": "a" * 40, "evidence_paths": ["observed.jsonl"]})
            record = json.loads(path.read_text())
            self.assertEqual(record["request_clocks"], {"first_request_utc": "2026-01-01T00:00:01+00:00",
                                                        "last_request_utc": "2026-01-01T00:00:04+00:00"})
            events = [json.loads(line) for line in (root / "phases.jsonl").read_text().splitlines()]
            self.assertEqual([event["phase"] for event in events], ["first-request", "last-request"])
            self.assertEqual(events[0]["utc"], record["request_clocks"]["first_request_utc"])
            self.assertEqual(evidence.request_clocks(root, []), {"first_request_utc": None, "last_request_utc": None})

    def test_interruption_keeps_unknown_counts(self):
        import failure_writer
        with tempfile.TemporaryDirectory() as temporary:
            argv = ["failure_writer", "--out", temporary, "--id", "BC-GPU-001", "--sha", "a" * 40,
                    "--start", "2026-01-01T00:00:00Z", "--reason", "runner interrupted", "--type", "infra_interrupted"]
            with patch("sys.argv", argv), patch.dict("os.environ", {}, clear=True):
                failure_writer.main()
            record = json.loads((Path(temporary) / "results/BC-GPU-001.json").read_text())
            self.assertEqual(record["infra_history"][0]["type"], "infra_interrupted")
            self.assertIsNone(record["observed_values"]["request_count"])
            self.assertIsNone(record["observed_values"]["error_count"])

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
