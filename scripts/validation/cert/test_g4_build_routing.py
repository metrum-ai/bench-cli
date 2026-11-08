# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Mock scheduling/proof tests only; never real GPU output or certification."""
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch
import g4_edge_runner as runner
from execution_evidence import digest


class G4BuildRoutingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.args = SimpleNamespace(out=root / 'out', candidate=root / 'shipped', baseline=root / 'baseline',
            manifest=root / 'lane.json', sha='a' * 40, model='synthetic', revision='b' * 40,
            image='synthetic', start_utc='2026-01-01T00:00:00Z', owner='test', evidence_link='synthetic')
        for path in (self.args.candidate, self.args.baseline):
            path.write_text('SYNTHETIC NON-EXECUTABLE ' + path.name)
        self.evidence = self.args.out / 'evidence'
        self.evidence.mkdir(parents=True)
        (self.evidence / 'gpu.csv').write_text('synthetic-gpu,synthetic-driver\n')
        tokenizer = root / 'tokenizer.json'
        tokenizer.write_text('{"synthetic":true}')
        self.args.manifest.write_text(json.dumps({'lanes': {'G4': {'tokenizer': {'path': str(tokenizer), 'sha256': digest(tokenizer)}}}}))

    def test_runner_uses_shipped_build_for_all_cells_and_binds_proof_before_run_cell(self):
        proofs = {}
        def select(manifest, name):
            path = self.args.candidate
            proof = {'binary_sha256': digest(path), 'label': 'shipped build (otlp,tokenizer)'}
            proofs[name] = proof
            return path, proof
        def execute(args, label, binary, count, cap, thinking, extra, **kwargs):
            name = label.removeprefix('g4-edge-')
            self.assertEqual(binary, args.candidate)
            self.assertEqual(json.loads((self.evidence / (label + '-build.json')).read_text()), proofs[name])
            self.assertEqual((count, cap, kwargs['concurrency']), (4, 128, 1))
            self.assertEqual(thinking, name == 'truncated')
            self.assertEqual(kwargs['fault'], 'remove_usage' if name in ('missing', 'fallback') else None)
            self.assertEqual('--tokenizer' in extra, name == 'fallback')
            return '', '', 0, None
        frozen = {'candidate': {'sha256': digest(self.args.candidate)}}
        with patch.object(runner.argparse.ArgumentParser, 'parse_args', return_value=self.args), \
             patch.object(runner.gpu010_builds, 'from_environment', return_value=frozen), \
             patch.object(runner.gpu010_builds, 'cell', side_effect=select), \
             patch.object(runner.r_runner, 'run_cell', side_effect=execute) as run, \
             patch.object(runner.g4_edge_checks, 'all_truncated', return_value={'mock-only': True}), \
             patch.object(runner.g4_edge_checks, 'empty', return_value={'mock-only': True}), \
             patch.object(runner.g4_edge_checks, 'missing', side_effect=lambda *a: {'mock-only': True}), \
             patch.object(runner.result_writer, 'write_record') as write:
            self.assertEqual(runner.main(), 0)
        self.assertEqual(run.call_count, 4)
        record = write.call_args.args[1]
        self.assertEqual(set(record['observed_values']['cells']), set(runner.gpu010_builds.CELLS))
        for name in runner.gpu010_builds.CELLS:
            self.assertEqual(record['observed_values']['cells'][name]['build'], proofs[name])
        self.assertEqual(record['versions']['candidate_binary_sha256'], digest(self.args.candidate))
        self.assertEqual(record['versions']['fallback_binary_sha256'], digest(self.args.candidate))
        self.assertEqual(record['versions']['fallback_build_label'], 'shipped build (otlp,tokenizer)')
        self.assertEqual(record['thresholds'], {'requests_per_live_cell': 4, 'empty_issuance_count': 0, 'legacy_rate_tolerance_pct': 1})

    def test_missing_provenance_blocks_every_cell_before_any_measured_request(self):
        with patch.object(runner.argparse.ArgumentParser, 'parse_args', return_value=self.args), \
             patch.object(runner.gpu010_builds, 'from_environment', side_effect=ValueError('missing proof')), \
             patch.object(runner.r_runner, 'run_cell') as run, \
             patch.object(runner.result_writer, 'write_record') as write:
            self.assertEqual(runner.main(), 1)
        run.assert_not_called()
        record = write.call_args.args[1]
        self.assertEqual(record['result'], 'FAIL')
        self.assertTrue(set(runner.gpu010_builds.CELLS).issubset(record['observed_values']['failed_cells']))


if __name__ == '__main__':
    unittest.main()
