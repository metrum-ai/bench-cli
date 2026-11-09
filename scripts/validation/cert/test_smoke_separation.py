# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic smoke-only classification controls; no smoke was executed."""
import datetime as dt
import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import result_writer
import selected_set
import validate_results


class SmokeSeparationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.smoke = self.root / 'smoke'
        self.smoke.mkdir()
        self.attempt = self.smoke / 'attempt'
        self.attempt.mkdir()
        self.marker = self.smoke / result_writer.SMOKE_MARKER
        self.marker.write_text(json.dumps({'classification': result_writer.SMOKE_CLASS,
                                           'expectations_sha256': 'a' * 64}))
        self.record = {'ID': 'BC-GPU-001', 'candidate_sha': 'b' * 40, 'result': 'FAIL'}

    def test_writer_labels_record_and_certification_validator_rejects_it(self):
        with patch.object(result_writer, 'enrich', side_effect=lambda _, record: record):
            output = result_writer.write_record(self.attempt, self.record)
        saved = json.loads(output.read_text())
        self.assertEqual(saved['evidence_class'], result_writer.SMOKE_CLASS)
        self.assertEqual(saved['stage0_expectations_sha256'], 'a' * 64)
        # Even if a smoke result is presented as PASS in a different root,
        # it is never accepted while its immutable classification is retained.
        saved['result'] = 'PASS'
        with self.assertRaisesRegex(ValueError, 'non-certification evidence'):
            validate_results.check_record(saved, 'b' * 40, self.root, dt.datetime.now(dt.timezone.utc))

    def test_smoke_record_is_rejected_from_certification_attempt_history(self):
        catalog = self.root / 'catalog.csv'
        catalog.write_text('ID,Tier,Test case\nBC-GPU-001,PR,fixture\n')
        results = self.root / 'cert-results'
        history = self.root / 'cert-history'
        results.mkdir(); history.mkdir()
        (self.root / 'evidence.txt').write_text('fixture')
        current = {'ID': 'BC-GPU-001', 'candidate_sha': 'b' * 40, 'result': 'PASS', 'attempt': 2,
                   'owner': 'fixture', 'evidence_link': 'synthetic', 'observed_values': {'n': 1},
                   'thresholds': {'n': 1}, 'evidence_paths': ['evidence.txt'],
                   'start_utc': '2026-01-01T00:00:00Z', 'end_utc': '2026-01-01T00:01:00Z'}
        (results / 'BC-GPU-001.json').write_text(json.dumps(current))
        previous = {**self.record, 'attempt': 1, 'evidence_class': result_writer.SMOKE_CLASS,
                    'stage0_expectations_sha256': 'a' * 64}
        (history / 'attempt-1.json').write_text(json.dumps(previous))
        with self.assertRaisesRegex(validate_results.NonCertificationEvidence, 'non-certification evidence'):
            validate_results.validate(catalog, results, 'b' * 40,
                dt.datetime(2026, 11, 1, tzinfo=dt.timezone.utc), history=history)

    def test_marked_smoke_root_is_rejected_even_without_records(self):
        catalog = self.root / 'catalog.csv'
        catalog.write_text('ID,Tier,Test case\nBC-GPU-001,PR,fixture\n')
        results = self.attempt / 'results'
        results.mkdir()
        with self.assertRaisesRegex(validate_results.NonCertificationEvidence, 'marked smoke root'):
            validate_results.validate(catalog, results, 'b' * 40, dt.datetime.now(dt.timezone.utc))
        sheet, report = self.root / 'sheet.csv', self.root / 'report.md'
        cmd = [sys.executable, str(Path(validate_results.__file__)), '--catalog', str(catalog),
               '--results', str(results), '--candidate-sha', 'b' * 40, '--release', 'v1.9.0',
               '--cadence', 'weekly', '--run-id', 'fixture', '--run-date-utc', '2026-11-01T00:00:00Z',
               '--branch', 'fixture', '--compared-against', 'fixture', '--csv', str(sheet), '--markdown', str(report)]
        run = subprocess.run(cmd, capture_output=True, text=True, check=False)
        self.assertEqual(run.returncode, 2)
        self.assertIn('marked smoke root', run.stderr)
        self.assertFalse(sheet.exists())
        self.assertFalse(report.exists())

    def test_marked_root_rejected_for_selected_set_na_even_without_record_label(self):
        results = self.attempt / 'results'
        results.mkdir()
        path = results / 'BC-GPU-001.json'
        # Synthetic fixture, never a real smoke output or certification attempt.
        record = {'ID': 'BC-GPU-001', 'candidate_sha': 'b' * 40, 'result': 'N-A',
                  'approval': {'scope': 'BC-GPU-001'}}
        path.write_text(json.dumps(record))
        index = {'approved_na': [{'record': {'path': str(path), 'sha256': hashlib.sha256(path.read_bytes()).hexdigest()},
                                  'evidence_root': str(self.attempt), 'evidence': {}}]}
        with self.assertRaisesRegex(validate_results.NonCertificationEvidence, 'marked smoke root'):
            selected_set.approved_na_cases(index, {'BC-GPU-001'}, 'b' * 40)

    def test_nested_smoke_evidence_and_history_abort_inventory(self):
        catalog = self.root / 'catalog.csv'
        catalog.write_text('ID,Tier,Test case,Cadence\nBC-GPU-001,PR,fixture,weekly\n')
        results = self.root / 'cert-results'
        results.mkdir()
        raw = self.attempt / 'raw.txt'
        raw.write_text('synthetic fixture')
        record = {'ID': 'BC-GPU-001', 'candidate_sha': 'b' * 40, 'result': 'PASS', 'attempt': 1,
                  'owner': 'fixture', 'evidence_link': 'synthetic', 'observed_values': {'n': 1},
                  'thresholds': {'n': 1}, 'evidence_paths': ['smoke/attempt/raw.txt'],
                  'start_utc': '2026-01-01T00:00:00Z', 'end_utc': '2026-01-01T00:01:00Z'}
        (results / 'BC-GPU-001.json').write_text(json.dumps(record))
        with self.assertRaisesRegex(validate_results.NonCertificationEvidence, 'marked smoke root'):
            validate_results.validate(catalog, results, 'b' * 40, dt.datetime.now(dt.timezone.utc))
        sheet, report = self.root / 'sheet.csv', self.root / 'report.md'
        cmd = [sys.executable, str(Path(validate_results.__file__)), '--catalog', str(catalog),
               '--results', str(results), '--candidate-sha', 'b' * 40, '--release', 'v1.9.0',
               '--cadence', 'weekly', '--run-id', 'fixture', '--run-date-utc', '2026-11-01T00:00:00Z', '--branch', 'fixture',
               '--compared-against', 'fixture', '--csv', str(sheet), '--markdown', str(report)]
        run = subprocess.run(cmd, capture_output=True, text=True, check=False)
        self.assertEqual(run.returncode, 2, run.stderr)
        self.assertIn('marked smoke root', run.stderr)
        self.assertFalse(sheet.exists())
        self.assertFalse(report.exists())
        history = self.root / 'history'
        nested = history / 'nested'
        nested.mkdir(parents=True)
        (nested / result_writer.SMOKE_MARKER).write_text(self.marker.read_text())
        (nested / 'prior.json').write_text(json.dumps(self.record))
        with self.assertRaisesRegex(validate_results.NonCertificationEvidence, 'marked smoke root'):
            validate_results.validate(catalog, results, 'b' * 40, dt.datetime.now(dt.timezone.utc), history=history)

    def test_unimplemented_lane_fallback_labels_every_failure(self):
        binary = self.root / 'binary'
        binary.write_text('fixture')
        manifest = self.root / 'manifest.json'
        manifest.write_text('{}')
        out = self.smoke / 'unimplemented'
        lane = Path(result_writer.__file__).with_name('lane.py')
        cmd = [sys.executable, str(lane), 'G1', '--candidate', str(binary), '--baseline', str(binary),
               '--manifest', str(manifest), '--out', str(out)]
        run = subprocess.run(cmd, capture_output=True, text=True, check=False)
        self.assertEqual(run.returncode, 1, run.stderr)
        outputs = sorted((out / 'results').glob('*.json'))
        self.assertTrue(outputs)
        for path in outputs:
            record = json.loads(path.read_text())
            self.assertEqual(record['result'], 'FAIL')
            self.assertEqual(record['evidence_class'], result_writer.SMOKE_CLASS)
            self.assertEqual(record['stage0_expectations_sha256'], 'a' * 64)

    def test_invalid_or_nested_markers_and_unmarked_smoke_fail_closed(self):
        self.marker.write_text('{"classification":"invalid"}')
        with self.assertRaisesRegex(ValueError, 'invalid smoke root'):
            result_writer.classify_smoke(self.attempt, self.record)
        self.marker.write_text(json.dumps({'classification': result_writer.SMOKE_CLASS,
                                          'expectations_sha256': 'a' * 64}))
        (self.attempt / result_writer.SMOKE_MARKER).write_text(self.marker.read_text())
        with self.assertRaisesRegex(ValueError, 'nested smoke roots'):
            result_writer.classify_smoke(self.attempt, self.record)
        (self.attempt / result_writer.SMOKE_MARKER).unlink()
        self.marker.unlink()
        with self.assertRaisesRegex(ValueError, 'marked smoke root'):
            result_writer.classify_smoke(self.attempt, {**self.record, 'evidence_class': result_writer.SMOKE_CLASS})
        self.assertEqual(result_writer.classify_smoke(self.attempt, self.record), self.record)

    def test_marker_cannot_override_record_or_reference_symlink(self):
        with self.assertRaisesRegex(ValueError, 'controlled by its root'):
            result_writer.classify_smoke(self.attempt, {**self.record, 'stage0_expectations_sha256': 'b' * 64})
        target = self.root / 'target.json'
        target.write_text(self.marker.read_text())
        self.marker.unlink()
        self.marker.symlink_to(target)
        with self.assertRaisesRegex(ValueError, 'symlink'):
            result_writer.classify_smoke(self.attempt, self.record)


if __name__ == '__main__':
    unittest.main()
