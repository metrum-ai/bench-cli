# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Fail-closed, opt-in cross-candidate evidence carry-over."""
import datetime as dt
import hashlib
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import validate_results as cert


class CarryOverTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.git("init", "-q")
        self.git("config", "user.name", "Test")
        self.git("config", "user.email", "test@example.invalid")
        self.git("commit", "--allow-empty", "-qm", "source")
        self.source = self.git("rev-parse", "HEAD")
        self.git("commit", "--allow-empty", "-qm", "target")
        self.target = self.git("rev-parse", "HEAD")
        self.unrelated = self.git("commit-tree", "HEAD^{tree}", "-m", "unrelated")
        self.patch = mock.patch.object(cert, "REPO", self.repo)
        self.patch.start()
        self.addCleanup(self.patch.stop)
        self.results = self.root / "results"
        self.results.mkdir()
        self.catalog = self.root / "catalog.csv"
        self.set_case("BC-CLI-001")
        self.evidence = self.root / "case-evidence"
        self.evidence.mkdir()
        (self.evidence / "raw.txt").write_text("real evidence")
        (self.root / "proof.txt").write_text("reviewed diff proof")
        self.now = dt.datetime(2026, 11, 6, tzinfo=dt.timezone.utc)
        self.record = {"ID": "BC-CLI-001", "candidate_sha": self.source, "result": "PASS", "attempt": 1,
                       "owner": "tester", "evidence_link": "private:evidence", "evidence_paths": ["raw.txt"],
                       "start_utc": "2026-11-04T10:00:00Z", "end_utc": "2026-11-04T10:01:00Z",
                       "observed_values": {"count": 1}, "thresholds": {"minimum": 1}}
        self.manifest = self.root / "carry-over-manifest.json"

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.repo), *args], text=True).strip()

    def set_case(self, case):
        self.catalog.write_text("ID,Tier,Test case\n" + case + ",PR,Case\n", encoding="utf-8")

    def setup_record(self, record=None):
        record = self.record if record is None else record
        path = self.results / (record["ID"] + ".json")
        path.write_bytes((json.dumps(record, sort_keys=True) + "\n").encode())
        entry = {"ID": record["ID"], "attempt": record["attempt"], "source_sha": record["candidate_sha"],
                 "record_sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "evidence_root": "case-evidence",
                 "proofs": [{"path": "proof.txt", "sha256": hashlib.sha256((self.root / "proof.txt").read_bytes()).hexdigest()}],
                 "disposition": {"applicable": True, "reviewer": "Master", "utc": "2026-11-05T12:00:00Z", "reason": "reviewed case scope"}}
        return path, entry

    def validate(self, entries, *, target=None, history=None):
        self.manifest.write_text(json.dumps({"target_candidate_sha": target or self.target, "entries": entries}))
        return cert.validate(self.catalog, self.results, self.target, self.now, history=history,
                             carry_over_manifest=self.manifest)

    def test_unchanged_original_needs_opt_in(self):
        path, entry = self.setup_record()
        original = path.read_bytes()
        self.assertTrue(cert.validate(self.catalog, self.results, self.target, self.now)[1])
        table, errors = self.validate([entry])
        self.assertEqual([], errors)
        self.assertEqual("PASS", table[0]["Result"])
        self.assertIn(self.source, table[0]["Notes"])
        self.assertEqual(original, path.read_bytes())

    def test_missing_case_attempt_and_extra_entry_fail(self):
        _, entry = self.setup_record()
        for field, value in (("ID", "BC-CLI-002"), ("attempt", 2)):
            with self.subTest(field=field):
                self.assertTrue(self.validate([{**entry, field: value}])[1])
        with self.assertRaises(ValueError):
            self.validate([entry, entry])

    def test_target_record_hash_and_source_checks(self):
        path, entry = self.setup_record()
        with self.assertRaises(ValueError):
            self.validate([entry], target="f" * 40)
        path.write_bytes(path.read_bytes() + b" ")
        self.assertTrue(self.validate([entry])[1])
        _, entry = self.setup_record({**self.record, "candidate_sha": self.unrelated})
        self.assertTrue(self.validate([entry])[1])
        _, entry = self.setup_record()
        self.assertTrue(self.validate([{**entry, "source_sha": self.unrelated}])[1])

    def test_proofs_missing_hash_and_path_escape_fail(self):
        _, entry = self.setup_record()
        for proofs in ([], [{"path": "missing", "sha256": "0" * 64}],
                       [{"path": "proof.txt", "sha256": "0" * 64}],
                       [{"path": "../outside", "sha256": "0" * 64}]):
            with self.subTest(proofs=proofs):
                self.assertTrue(self.validate([{**entry, "proofs": proofs}])[1])
        (self.root / "proof.txt").write_text("tampered")
        self.assertTrue(self.validate([entry])[1])
        self.assertTrue(self.validate([{**entry, "evidence_root": "../outside"}])[1])

    def test_disposition_is_required_and_not_future(self):
        _, entry = self.setup_record()
        for change in ({"applicable": False}, {"applicable": 1}, {"reviewer": ""}, {"reason": ""},
                       {"utc": "2026-11-07T00:00:00Z"}, {"utc": "invalid"}):
            with self.subTest(change=change):
                self.assertTrue(self.validate([{**entry, "disposition": {**entry["disposition"], **change}}])[1])

    def test_every_must_rerun_case_rejected(self):
        for case in sorted(cert.MUST_RERUN):
            with self.subTest(case=case):
                self.set_case(case)
                record = {**self.record, "ID": case}
                path, entry = self.setup_record(record)
                self.assertTrue(self.validate([entry])[1])
                path.unlink()
                current = {**record, "candidate_sha": self.target, "evidence_paths": ["case-evidence/raw.txt"]}
                self.setup_record(current)
                self.assertFalse(cert.validate(self.catalog, self.results, self.target, self.now)[1])
                (self.results / (case + ".json")).unlink()

    def test_carried_history_requires_exact_manifest_entry(self):
        history = self.root / "history"
        history.mkdir()
        previous = {**self.record, "result": "FAIL"}
        path, entry = self.setup_record(previous)
        path.rename(history / "attempt-1.json")
        current = {**self.record, "candidate_sha": self.target, "attempt": 2,
                   "evidence_paths": ["case-evidence/raw.txt"]}
        self.setup_record(current)
        self.assertFalse(self.validate([entry], history=history)[1])
        self.assertTrue(cert.validate(self.catalog, self.results, self.target, self.now, history=history)[1])
        (history / "attempt-1.json").write_text("{}")
        self.assertTrue(self.validate([entry], history=history)[1])


if __name__ == "__main__":
    unittest.main()
