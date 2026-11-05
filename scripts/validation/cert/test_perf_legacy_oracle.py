# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic independent reconstruction tests, not GPU evidence."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

import perf_legacy_oracle as oracle

PERF = Path(__file__).resolve().parent.parent / 'perf/test_compare.py'
spec = importlib.util.spec_from_file_location('perf_fixtures', PERF)
fixtures = importlib.util.module_from_spec(spec); spec.loader.exec_module(fixtures)


class LegacyOracleTests(unittest.TestCase):
    def test_grid_matches_comparator_and_detects_tampering(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); baseline = root / 'bench-rel'; candidate = root / 'bench-src'
            for workload in ('W1', 'W2'):
                for concurrency in (1, 8, 32):
                    for directory in (baseline, candidate):
                        point = fixtures.write_point(directory, workload, concurrency, 1, tok_s=100, tpot_ms=40)
                        data = point / 'requests.jsonl'
                        rows = [json.loads(line) for line in data.read_text().splitlines()]
                        rows[-1].update({'attempted': 8, 'successes': 8, 'errors': 0, 'usage_missing_count': 0,
                                         'completion_tokens_total': 24, 'prompt_tokens_total': 800,
                                         'completion_tokens_per_second': 100, 'ttft_coverage': {'measured': 8},
                                         'ttft_first_any_s': {'n': 8, 'p50': .05}})
                        data.write_text(''.join(json.dumps(row) + '\n' for row in rows))
            code, _, report = fixtures.run_compare(baseline, candidate)
            self.assertEqual(code, 0)
            self.assertEqual(oracle.verify_grid(root, report, 'q38-27b')['result'], 'PASS')
            report['points'][0]['candidate']['system_output_tok_s'] += 1
            with self.assertRaises(ValueError): oracle.verify_grid(root, report, 'q38-27b')

    def test_missing_summary_coverage_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            point = fixtures.write_point(tmp, 'W1', 1, 1)
            with self.assertRaises(ValueError): oracle.point(point)


if __name__ == '__main__': unittest.main()
