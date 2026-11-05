# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline, CPU-only tests for certification evidence inventory."""
import datetime as dt
import io
import json
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import check_live_docs
import check_package_docs
import select_cases
import validate_results as cert


class ValidatorTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.results = self.root / "results"
        self.results.mkdir()
        self.catalog = self.root / "catalog.csv"
        self.catalog.write_text("ID,Tier,Test case,Cadence,Parallel group,Area,Selection\nBC-CLI-001,PR,Flags,weekly,MOCK,CLI,core\nBC-POST-001,POST,Published,,POST,POST,core\n", encoding="utf-8")
        (self.root / "evidence.txt").write_text("evidence", encoding="utf-8")
        self.sha = "a" * 40
        self.selection = self.root / "selection.json"
        selection = select_cases.make_selection(cert.load_catalog(self.catalog), select_cases.catalog_hash(self.catalog), self.sha, .3, 3, [], [])
        self.selection.write_text(json.dumps(selection), encoding="utf-8")
        self.now = dt.datetime(2026, 11, 5, tzinfo=dt.timezone.utc)
        self.record = {"ID": "BC-CLI-001", "candidate_sha": self.sha, "result": "PASS",
                       "observed_values": {"count": 1}, "thresholds": {"minimum": 1},
                       "evidence_paths": ["evidence.txt"], "evidence_link": "https://example.test/evidence", "owner": "tester", "attempt": 1,
                       "start_utc": "2026-11-04T12:00:00Z", "end_utc": "2026-11-04T12:01:00Z"}

    def run_record(self, record):
        (self.results / "BC-CLI-001.json").write_text(json.dumps(record), encoding="utf-8")
        return cert.validate(self.catalog, self.results, self.sha, self.now)

    def test_valid_pass(self):
        table, errors = self.run_record(self.record)
        self.assertEqual([], errors)
        self.assertEqual("PASS", table[0]["Result"])

    def test_missing_fails_and_post_is_excluded(self):
        table, errors = cert.validate(self.catalog, self.results, self.sha, self.now)
        self.assertTrue(errors)
        self.assertEqual(["BC-CLI-001"], [r["ID"] for r in table])

    def test_fail_and_mismatched_candidate_fail(self):
        for change in ({"result": "FAIL"}, {"candidate_sha": "b" * 40}, {"observed_values": {}}, {"attempt": True}):
            with self.subTest(change=change):
                self.assertTrue(self.run_record({**self.record, **change})[1])

    def test_evidence_cannot_escape_or_be_missing(self):
        for name in ("../outside", "missing", "/etc/passwd"):
            with self.subTest(name=name):
                self.assertTrue(self.run_record({**self.record, "evidence_paths": [name]})[1])

    def test_approved_na_independent_and_prospective(self):
        record = {"ID": "BC-CLI-001", "candidate_sha": self.sha, "result": "N-A",
                  "case_start_utc": "2026-11-04T12:00:00Z", "evidence_paths": ["evidence.txt"],
                  "owner": "owner", "evidence_link": "https://example.test/approval",
                  "approval": {"reason": "not applicable", "requester": "owner", "approver": "reviewer",
                               "scope": "this case", "approved_utc": "2026-11-04T11:00:00Z",
                               "expires_utc": "2026-11-06T00:00:00Z"}}
        self.assertFalse(self.run_record(record)[1])
        record["approval"]["approver"] = "OWNER"
        self.assertTrue(self.run_record(record)[1])
        record["approval"]["approver"] = "reviewer"
        record["approval"]["approved_utc"] = "2026-11-04T13:00:00Z"
        self.assertTrue(self.run_record(record)[1])

    def test_unexpected_json_file_and_duplicate_key_fail(self):
        self.run_record(self.record)
        (self.results / "extra.json").write_text('{"ID":"BC-CLI-001","ID":"BC-CLI-001"}', encoding="utf-8")
        self.assertTrue(cert.validate(self.catalog, self.results, self.sha, self.now)[1])

    def test_result_and_periodic_sheet_headers_and_report(self):
        import csv
        self.run_record(self.record)
        for mode in ("release", "weekly"):
            with self.subTest(mode=mode):
                dest = self.root / (mode + ".csv")
                report = self.root / (mode + ".md")
                args = [sys.executable, str(Path(cert.__file__)), "--catalog", str(self.catalog),
                        "--results", str(self.results), "--candidate-sha", self.sha, "--release", "v1.8.0",
                        "--csv", str(dest), "--markdown", str(report)]
                if mode == "weekly":
                    args += ["--cadence", "weekly", "--run-id", "run-1", "--run-date-utc", "2026-11-05T00:00:00Z",
                             "--branch", "main", "--compared-against", "v1.7.2"]
                else:
                    args += ["--selection", str(self.selection)]
                result = subprocess.run(args, check=False, capture_output=True, text=True)
                self.assertEqual(0, result.returncode, result.stderr)
                with dest.open(newline="", encoding="utf-8") as stream:
                    reader = csv.DictReader(stream)
                    self.assertEqual(cert.PERIODIC_FIELDS if mode == "weekly" else cert.RESULT_FIELDS, reader.fieldnames)
                    self.assertEqual("PASS", next(reader)["Result"])
                self.assertIn("Lane summary", report.read_text(encoding="utf-8"))
                self.assertIn("BC-CLI-001", report.read_text(encoding="utf-8"))

    def test_later_attempt_requires_complete_history(self):
        later = {**self.record, "attempt": 2}
        self.assertTrue(self.run_record(later)[1])
        history = self.root / "history"
        history.mkdir()
        previous = {**self.record, "result": "FAIL", "reason": "earlier failure"}
        (history / "attempt-1.json").write_text(json.dumps(previous), encoding="utf-8")
        table, errors = cert.validate(self.catalog, self.results, self.sha, self.now, history=history)
        self.assertFalse(errors)
        self.assertEqual("FAIL", table[0]["History"][0]["result"])

    def test_duplicate_catalog_id_rejected(self):
        self.catalog.write_text("ID,Tier,Test case\nBC-CLI-001,PR,One\nBC-CLI-001,RC,Two\n", encoding="utf-8")
        with self.assertRaises(ValueError):
            cert.load_catalog(self.catalog)


class PackageDocsTests(unittest.TestCase):
    def make_archive(self, root, doc_content=b"ok", include_doc=True):
        path = root / "fixture.tar.gz"
        with tarfile.open(path, "w:gz") as archive:
            entries = [("release/README.md", b"[guide](docs/guide.md)" )]
            if include_doc:
                entries.append(("release/docs/guide.md", doc_content))
            for name, content in entries:
                member = tarfile.TarInfo(name)
                member.size = len(content)
                archive.addfile(member, io.BytesIO(content))
        return path

    def test_byte_identical_docs_and_link_required(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            check_package_docs.check_docs(self.make_archive(root), {"docs/guide.md": b"ok"})
            with self.assertRaises(ValueError):
                check_package_docs.check_docs(self.make_archive(root, b"bad"), {"docs/guide.md": b"ok"})
            with self.assertRaises(ValueError):
                check_package_docs.check_docs(self.make_archive(root, include_doc=False), {"docs/guide.md": b"ok"})


class LiveDocsTests(unittest.TestCase):
    def test_scope_rejects_external_and_credentials(self):
        self.assertTrue(check_live_docs.in_scope(check_live_docs.START))
        self.assertFalse(check_live_docs.in_scope("https://elsewhere.test/metrum-ai-bench-cli/"))
        self.assertFalse(check_live_docs.in_scope("https://user@docs.metrum.ai/metrum-ai-bench-cli/"))
        self.assertFalse(check_live_docs.in_scope("http://docs.metrum.ai/metrum-ai-bench-cli/"))

    def test_crawl_ignores_external_links_and_reports_failures(self):
        class Response:
            headers = mock.Mock()
            headers.get_content_type.return_value = "text/html"
            def __enter__(self):
                return self
            def __exit__(self, *args):
                return False
            def geturl(self):
                return check_live_docs.START
            def read(self, size):
                return b'<a href="/metrum-ai-bench-cli/broken">bad</a><a href="https://elsewhere.test/">external</a>'
        opener = mock.Mock()
        opener.open.side_effect = [Response(), OSError("not found")]
        with mock.patch.object(check_live_docs, "build_opener", return_value=opener):
            seen, errors = check_live_docs.crawl()
        self.assertEqual(2, len(seen))
        self.assertEqual(1, len(errors))
        self.assertEqual(2, opener.open.call_count)


if __name__ == "__main__":
    unittest.main()
