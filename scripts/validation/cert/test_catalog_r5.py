# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Core r5 selection, unresolved pins and fail-closed release intake."""
import csv
import datetime as dt
import hashlib
import importlib
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import select_cases
import validate_results as validator

REPO = Path(__file__).resolve().parents[3]
CATALOG = REPO / 'docs/RELEASE_CERTIFICATION_CATALOG.csv'
CASES = {f'BC-KV-{n:03}' for n in range(1, 16)}


class CatalogR5Tests(unittest.TestCase):
    def test_lmcache_mp_mode_has_distinct_units_and_recipe(self):
        row = validator.load_catalog(CATALOG)['BC-KV-005']
        self.assertIn('connector_mode=mp', row['Model/server'])
        self.assertIn('LMCacheMPConnector', row['Model/server'])
        for flag in ('mamba-cache-mode align', 'prefix caching', 'separate-object-groups'):
            self.assertIn(flag, row['Command or procedure'])
        self.assertIn('reuse/lookup-hit delta', row['Pass criteria'])
        self.assertIn('lmcache_mp_*', row['Pass criteria'])
        self.assertIn('no stored-token or occupancy value is inferred', row['Pass criteria'])

    def test_rows_core_tier_cadence_and_r4_unchanged(self):
        rows = validator.load_catalog(CATALOG)
        self.assertEqual(CASES, {case for case in rows if case.startswith('BC-KV-')})
        for n in range(1, 16):
            row = rows[f'BC-KV-{n:03}']
            self.assertEqual('core', row['Selection'])
            self.assertEqual('per-release', row['Cadence'])
            self.assertEqual('RC-GPU' if n < 10 else 'RC', row['Tier'])
        existing = b''.join(line for line in CATALOG.read_bytes().splitlines(keepends=True)
                            if not line.startswith(b'BC-KV-'))
        # Frozen r4 bytes at 5a869da45e3a11e2b3465102521559b11f43a56b;
        # usable with a shallow CI checkout as well as a full clone.
        self.assertEqual('2b8b6ea76add584f7263a41bffe0ae6e5edd583a22946188810ac7b8b329de87',
                         hashlib.sha256(existing).hexdigest())

    def test_all_selected_even_with_complete_history_and_low_sample_rate(self):
        rows = validator.load_catalog(CATALOG)
        selection = select_cases.make_selection(rows, select_cases.catalog_hash(CATALOG),
            'a' * 40, .01, 3, [], [{'release': 'prior', 'ran': sorted(rows)}])
        self.assertTrue(CASES <= set(selection['selected']))
        for case in sorted(CASES):
            self.assertNotIn('TBD-pin-not-executable', selection['selected'][case])

    def test_all_fifteen_r5_rows_have_resolved_executable_pins(self):
        rows = validator.load_catalog(CATALOG)
        for case in sorted(CASES):
            with self.subTest(case=case):
                validator.require_executable(rows[case])

    def test_reviewed_engine_pins_match_runner_sources(self):
        rows = validator.load_catalog(CATALOG)
        engines = (
            (('BC-KV-003', 'BC-KV-004'), 'kv_sglang_runner', 'sglang0523',
             'sha256:f7876bef696ccf9cf75a2bf7ccb82970f5fac577f082fb8ca95c9a59d2160eb5'),
            (('BC-KV-005',), 'kv_lmcache_runner', 'lmcache057-mp',
             'sha256:3be0a977f2aa4bf1acd29d5ae1e18716c6660c4733e1528caaafd758cb452cbb'),
            (('BC-KV-006',), 'kv_r5_dynamo_runner', 'dynamo162-kvbm', None),
            (('BC-KV-007',), 'kv_r5_trtllm_runner', 'trt-llm121', None),
        )
        for cases, module, profile, child in engines:
            runner = importlib.import_module(module)
            child = child or runner.CHILD
            model = getattr(runner, 'MODEL', 'Qwen/Qwen3.8-27B-FP8')
            revision = getattr(runner, 'MODEL_REVISION', 'c3a462cc3d38cf351f3b26cbd763469eeb74ef81')
            for case in cases:
                with self.subTest(case=case):
                    pin = rows[case]['Model/server']
                    for item in (runner.IMAGE, f'amd64 {child}',
                                 f'source {runner.REVISION}', f'profile {profile}',
                                 f'{model} revision {revision}'):
                        self.assertIn(item, pin)
                    if case == 'BC-KV-005':
                        self.assertIn('single-manifest image (no index)', pin)
                    else:
                        self.assertIn(f'index {runner.IMAGE}', pin)

    def test_unresolved_pin_in_any_field_still_blocks_selection_and_execution(self):
        rows = validator.load_catalog(CATALOG)
        for case in sorted(CASES):
            for field in ('Model/server', 'Notes'):
                with self.subTest(case=case, field=field):
                    bad = {**rows, case: {**rows[case], field: 'TBD-pin synthetic negative control'}}
                    selection = select_cases.make_selection(bad, select_cases.catalog_hash(CATALOG),
                        'a' * 40, .01, 3, [], [])
                    self.assertIn('TBD-pin-not-executable', selection['selected'][case])
                    with self.assertRaisesRegex(ValueError, 'not executable'):
                        validator.require_executable(bad[case])

    def test_each_r5_omission_from_selection_rejected(self):
        rows = validator.load_catalog(CATALOG)
        selection = select_cases.make_selection(rows, select_cases.catalog_hash(CATALOG),
            'a' * 40, .3, 3, [], [])
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'selection.json'
            for case in sorted(CASES):
                with self.subTest(case=case):
                    bad = json.loads(json.dumps(selection))
                    del bad['selected'][case]
                    path.write_text(json.dumps(bad))
                    with self.assertRaises(ValueError):
                        validator.load_selection(path, CATALOG, rows, 'a' * 40)

    @patch('execution_intake.ExecutionIndex')
    def test_every_missing_fail_pending_and_smoke_result_blocks(self, index):
        index.return_value.unused_errors.return_value = []
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            results = root / 'results'
            results.mkdir()
            (root / 'proof.txt').write_text('synthetic evidence')
            catalog = root / 'catalog.csv'
            rows = validator.load_catalog(CATALOG)
            # Resolved synthetic pins isolate result intake from the pin gate.
            with catalog.open('w', newline='') as stream:
                writer = csv.DictWriter(stream, fieldnames=list(next(iter(rows.values()))))
                writer.writeheader()
                for case in sorted(CASES):
                    writer.writerow({key: value.replace('TBD-pin', 'resolved-fixture')
                                     for key, value in rows[case].items()})
            rows = validator.load_catalog(catalog)
            sha = 'a' * 40
            selection = select_cases.make_selection(rows, select_cases.catalog_hash(catalog), sha, .3, 3, [], [])
            def record(case):
                return {'ID': case, 'candidate_sha': sha, 'result': 'PASS',
                    'observed_values': {'fixture': True}, 'thresholds': {'fixture': True},
                    'evidence_paths': ['proof.txt'], 'evidence_link': 'fixture',
                    'owner': 'fixture', 'attempt': 1, 'start_utc': '2026-01-01T00:00:00Z',
                    'end_utc': '2026-01-01T00:01:00Z'}
            for case in CASES:
                (results / f'{case}.json').write_text(json.dumps(record(case)))
            def validate():
                return validator.validate(catalog, results, sha, dt.datetime.now(dt.timezone.utc),
                    selection=selection, execution_index=root / 'synthetic-index',
                    execution_index_sha256='b' * 64, selection_sha256='c' * 64)
            self.assertEqual([], validate()[1])
            for case in sorted(CASES):
                path = results / f'{case}.json'
                for state in ('missing', 'FAIL', 'PENDING', 'smoke'):
                    with self.subTest(case=case, state=state):
                        path.unlink()
                        if state != 'missing':
                            rec = record(case)
                            if state == 'smoke':
                                rec['evidence_class'] = 'Stage-0 smoke, not certification'
                            else:
                                rec['result'] = state
                            path.write_text(json.dumps(rec))
                        if state == 'smoke':
                            with self.assertRaises(validator.NonCertificationEvidence):
                                validate()
                        else:
                            self.assertTrue(any(case in error for error in validate()[1]))
                        path.write_text(json.dumps(record(case)))
            # Even an apparently valid PASS cannot bypass an unresolved pin.
            with catalog.open('w', newline='') as stream:
                writer = csv.DictWriter(stream, fieldnames=list(next(iter(rows.values()))))
                writer.writeheader()
                for case in sorted(CASES):
                    writer.writerow({**rows[case], 'Model/server': 'TBD-pin synthetic negative control'})
            selection = select_cases.make_selection(validator.load_catalog(catalog),
                select_cases.catalog_hash(catalog), sha, .3, 3, [], [])
            self.assertTrue(any('TBD-pin' in error for error in validate()[1]))


if __name__ == '__main__':
    unittest.main()
