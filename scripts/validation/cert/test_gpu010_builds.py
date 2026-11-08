# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic GPU010 build-binding tests; not workflow or GPU evidence."""
import copy
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import gpu010_builds
import pretag_artifact
from build_role_test_fixture import bind
from execution_evidence import digest


class GPU010BuildTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.sha = 'a' * 40
        primary = self.root / 'shipped'
        primary.write_bytes(b'#!/bin/sh\n# synthetic shipped LLM\nexit 0\n')
        primary.chmod(0o755)
        self.manifest = {'candidate_sha': self.sha, 'candidate': {'path': str(primary), 'sha256': digest(primary)},
                         'cases': {'BC-GPU-010': {'command': ['synthetic-no-launch']}}}
        bind(self.manifest, self.root)
        item = self.manifest['candidate']
        self.manifest['inputs'] = [self.manifest['build_receipt'], pretag_artifact.pin(primary)] + [
            item['origin'][key] for key in gpu010_builds.ORIGIN_PINS]
        tokenizer = self.root / 'tokenizer.json'
        tokenizer.write_text('{"synthetic": true}')
        self.manifest['tokenizer'] = pretag_artifact.pin(tokenizer)

    def test_all_four_cells_bind_exact_shipped_workflow_binary(self):
        mapping = gpu010_builds.bindings(self.manifest)
        self.assertEqual(set(mapping), set(gpu010_builds.CELLS))
        for name in gpu010_builds.CELLS:
            path, proof = gpu010_builds.cell(self.manifest, name)
            self.assertEqual(mapping[name], self.manifest['candidate'])
            self.assertEqual(path, Path(self.manifest['candidate']['path']))
            self.assertEqual(proof['binary_sha256'], self.manifest['candidate']['sha256'])
            self.assertEqual(proof['features'], ['otlp', 'tokenizer'])
            self.assertEqual(proof['label'], 'shipped build (otlp,tokenizer)')
        with self.assertRaises(ValueError):
            gpu010_builds.cell(self.manifest, 'unknown')

    def test_other_cases_and_unapproved_variant_are_rejected(self):
        for cases in ({'BC-GPU-003': {}}, {'BC-GPU-010': {}, 'BC-GPU-009': {}}, {}):
            bad = copy.deepcopy(self.manifest); bad['cases'] = cases
            with self.subTest(cases=cases), self.assertRaises(ValueError):
                gpu010_builds.bindings(bad)
        bad = copy.deepcopy(self.manifest)
        bad['feature_variant_tokenizer'] = self.manifest['tokenizer_validation']
        with self.assertRaises(ValueError):
            gpu010_builds.bindings(bad)

    def synthetic_record(self):
        out = self.root / 'synthetic-output'; evidence = out / 'evidence'; evidence.mkdir(parents=True)
        cells = {}
        for name in gpu010_builds.CELLS:
            binary, proof = gpu010_builds.cell(self.manifest, name)
            prefix = evidence / ('g4-edge-' + name + '-')
            def write(suffix, value):
                Path(str(prefix) + suffix).write_text(json.dumps(value))
            write('build.json', proof); write('outcome.json', {'exit_code': 0})
            command = [str(binary), '--num-requests', '4', '--concurrency', '1', '--max-tokens', '128']
            if name == 'fallback': command += ['--tokenizer', self.manifest['tokenizer']['path']]
            if name == 'empty': command += ['--stop-after-seconds', '0']
            write('command.json', command)
            cells[name] = {'SYNTHETIC_REPLAY_MOCK_ONLY': True, 'build': proof}
        record = {'ID': 'BC-GPU-010', 'candidate_sha': self.sha, 'result': 'PASS',
                  'versions': {'candidate_binary_sha256': self.manifest['candidate']['sha256'],
                               'fallback_binary_sha256': self.manifest['candidate']['sha256'],
                               'fallback_build_label': 'shipped build (otlp,tokenizer)'},
                  'thresholds': {'requests_per_live_cell': 4, 'empty_issuance_count': 0, 'legacy_rate_tolerance_pct': 1},
                  'observed_values': {'cells': cells, 'failed_cells': []}}
        return out, record

    def test_replay_requires_all_cells_original_thresholds_and_raw_criteria(self):
        out, record = self.synthetic_record()
        with patch('g4_edge_checks.check_cell', return_value={'SYNTHETIC_REPLAY_MOCK_ONLY': True}) as replay:
            gpu010_builds.validate_record(self.manifest, record, out)
            self.assertEqual([call.args[0] for call in replay.call_args_list], list(gpu010_builds.CELLS))
            for field in ('cells', 'failed_cells'):
                bad = copy.deepcopy(record)
                if field == 'cells': del bad['observed_values']['cells']['fallback']
                else: bad['observed_values']['failed_cells'] = ['fallback']
                with self.subTest(field=field), self.assertRaises(ValueError):
                    gpu010_builds.validate_record(self.manifest, bad, out)
            bad = copy.deepcopy(record); bad['thresholds']['legacy_rate_tolerance_pct'] = 2
            with self.assertRaises(ValueError): gpu010_builds.validate_record(self.manifest, bad, out)
            with patch('g4_edge_checks.check_cell', side_effect=ValueError('unchanged criterion fails')), self.assertRaises(ValueError):
                gpu010_builds.validate_record(self.manifest, record, out)

    def test_replay_rejects_nonshipped_fallback_or_changed_proof(self):
        out, record = self.synthetic_record()
        command_path = out / 'evidence/g4-edge-fallback-command.json'
        command = json.loads(command_path.read_text()); command[0] = self.manifest['tokenizer_validation']['path']
        command_path.write_text(json.dumps(command))
        with patch('g4_edge_checks.check_cell', return_value={'SYNTHETIC_REPLAY_MOCK_ONLY': True}), self.assertRaisesRegex(ValueError, 'per-cell build'):
            gpu010_builds.validate_record(self.manifest, record, out)
        command[0] = self.manifest['candidate']['path']; command_path.write_text(json.dumps(command))
        proof = record['observed_values']['cells']['missing']['build']
        (out / 'evidence/g4-edge-fallback-build.json').write_text(json.dumps(proof))
        with patch('g4_edge_checks.check_cell', return_value={'SYNTHETIC_REPLAY_MOCK_ONLY': True}), self.assertRaisesRegex(ValueError, 'pre-request build proof'):
            gpu010_builds.validate_record(self.manifest, record, out)

    def test_direct_runner_requires_exact_case_environment_source_and_paths(self):
        source = self.root / 'per-case.json'; source.write_text(json.dumps(self.manifest))
        args = SimpleNamespace(sha=self.sha, candidate=Path(self.manifest['candidate']['path']),
                               manifest=Path(self.manifest['lane_manifest']['path']))
        env = {'CERT_CASE_ID': 'BC-GPU-010', 'CERT_INPUT_MANIFEST': str(source)}
        with patch.dict(gpu010_builds.os.environ, env, clear=True):
            self.assertEqual(gpu010_builds.from_environment(args), self.manifest)
            args.sha = 'b' * 40
            with self.assertRaises(ValueError): gpu010_builds.from_environment(args)
        for bad in ({}, {**env, 'CERT_CASE_ID': 'BC-GPU-009'}):
            with patch.dict(gpu010_builds.os.environ, bad, clear=True), self.assertRaises(ValueError):
                gpu010_builds.from_environment(args)

    def test_missing_each_workflow_pin_rejected(self):
        for index in range(len(self.manifest['inputs'])):
            bad = copy.deepcopy(self.manifest); del bad['inputs'][index]
            with self.subTest(pin=index), self.assertRaisesRegex(ValueError, 'frozen input pins'):
                gpu010_builds.bindings(bad)


if __name__ == '__main__':
    unittest.main()
