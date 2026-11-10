# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""KV envelope schema and semantic binding controls, entirely synthetic/offline."""
import copy
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import kv_envelope as envelope
from build_role_test_fixture import bind
from execution_evidence import digest


class EnvelopeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        binary = root / 'measured'
        binary.write_text('#!/bin/sh\n# synthetic unit fixture\nexit 0\n'); binary.chmod(0o755)
        def pin(path):
            return {'path': str(Path(path).resolve()), 'sha256': digest(path)}
        base = bind({'candidate_sha': 'a' * 40, 'candidate': pin(binary)}, root)
        spec = {'case': 'BC-KV-007', 'candidate_sha': base['candidate_sha'], 'tools': base['tools'],
                'timeout_seconds': 60, 'cleanup_grace_seconds': 10, 'cgroup_root': '/sys/fs/cgroup/synthetic'}
        Path(base['lane_manifest']['path']).write_text(json.dumps(spec))
        base['lane_manifest'] = pin(Path(base['lane_manifest']['path']))
        command = [str(envelope.HERE / 'case_entry.sh'), '--case', 'BC-KV-007', '--lane', 'kvt',
                   '--manifest', base['lane_manifest']['path'], '--manifest-sha256', base['lane_manifest']['sha256'],
                   '--out', str(root / 'new-attempt')]
        self.manifest = {key: base[key] for key in ('candidate_sha', 'candidate', 'build_receipt',
            'lane_manifest', 'tools', 'harness_repo', 'harness_closure')}
        self.manifest.update(schema_version='kv-r5-frozen.v1', lane='KVT', inputs=[],
            cases={'BC-KV-007': {'command': command, 'out': str(root / 'new-attempt'),
                                 'launcher': pin(envelope.HERE / 'case_entry.sh')}})
        inputs = {item['path']: pin(item['path']) for item in envelope.pins([
            self.manifest, spec, envelope.load(base['build_receipt']['path'])])}
        for path in (root / 'validation-only.log', envelope.SCHEMA):
            inputs[str(path)] = pin(path)
        self.manifest['inputs'] = list(inputs.values())
        self.verifier = patch('harness_integrity.verify', return_value=[])
        self.verifier.start(); self.addCleanup(self.verifier.stop)

    def test_complete_synthetic_envelope_and_case_mapping(self):
        self.assertEqual(self.manifest['cases'], envelope.validate(self.manifest, 'KVT'))

    def test_schema_missing_unknown_and_malformed_controls(self):
        for key in list(self.manifest):
            bad = copy.deepcopy(self.manifest); del bad[key]
            with self.subTest(missing=key), self.assertRaises(ValueError):
                envelope.validate_shape(bad)
        for change in ({'extra': True}, {'candidate_sha': 'main'}, {'lane': 'G1'}, {'cases': {}}, {'inputs': []}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                envelope.validate_shape({**self.manifest, **change})

    def test_source_tools_candidate_command_and_closure_controls(self):
        def command(b): b['cases']['BC-KV-007']['command'].append('--extra')
        def omit(b): b['inputs'].pop()
        def duplicate(b): b['inputs'].append(b['inputs'][0])
        def candidate(b): b['candidate']['role'] = 'validation-only'
        def tools(b): del b['tools']['python3']
        def path(b): b['candidate']['path'] = '/missing'
        def harness(b): b['harness_repo'] = '/wrong'
        for mutate in (command, omit, duplicate, candidate, tools, path, harness):
            bad = copy.deepcopy(self.manifest); mutate(bad)
            with self.subTest(control=mutate.__name__), self.assertRaises((ValueError, OSError)):
                envelope.validate(bad)

    def test_unresolved_row_and_cross_lane_rejected(self):
        bad = copy.deepcopy(self.manifest); bad['lane'] = 'KVV'
        with self.assertRaisesRegex(ValueError, 'mapping'):
            envelope.validate(bad)
        bad = copy.deepcopy(self.manifest); bad['lane'] = 'KVS'
        bad['cases']['BC-KV-003'] = bad['cases'].pop('BC-KV-007')
        # Keep the pin gate negative control independent of resolved live rows.
        import validate_results
        rows = validate_results.load_catalog(envelope.REPO / 'docs/RELEASE_CERTIFICATION_CATALOG.csv')
        rows['BC-KV-003']['Model/server'] = 'TBD-pin synthetic negative control'
        with patch.object(validate_results, 'load_catalog', return_value=rows), self.assertRaisesRegex(ValueError, 'TBD-pin'):
            envelope.validate(bad)

    def test_smoke_input_is_rejected_even_if_hash_matches(self):
        import result_writer
        root = Path(self.temp.name)
        (root / result_writer.SMOKE_MARKER).write_text(json.dumps({
            'classification': result_writer.SMOKE_CLASS, 'expectations_sha256': 'b' * 64}))
        # A smoke destination can use its own pins; certify destination elsewhere.
        bad = copy.deepcopy(self.manifest)
        bad['cases']['BC-KV-007']['out'] = str(root.parent / 'cert-attempt')
        bad['cases']['BC-KV-007']['command'][-1] = str(root.parent / 'cert-attempt')
        with self.assertRaises(result_writer.NonCertificationEvidence):
            envelope.validate(bad)


if __name__ == '__main__':
    unittest.main()
