# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic runtime envelopes; no actual generator, model or measured requests."""
import copy
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
import generated_inputs as generated
import runtime_inputs as runtime
import result_writer


class RuntimeInputsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        def asset(name):
            path = self.root / name
            path.write_text('SYNTHETIC INPUT ' + name)
            return runtime.pin(path)
        recipe = {'id': 'prompts', 'kind': generated.KIND, 'generator': asset('generator'),
                  'generator_version': '1.8.1', 'seed': 42, 'config': {'count': 4, 'profile': 'chat-short'},
                  'dataset': asset('dataset.jsonl'), 'tokenizer': {**asset('tokenizer.json'), 'revision': 'b' * 40},
                  'template': asset('template.txt')}
        receipt = self.root / 'receipt.json'
        receipt.write_text(json.dumps({'version': '1.8.1', 'builds': {'measured': {
            'binaries': {'metrum-ai-bench-cli-prompts': recipe['generator']}}}}))
        guard = patch.object(generated.build_roles, 'load_receipt')
        guard.start()
        self.addCleanup(guard.stop)
        lane = self.root / 'lane.json'
        lane.write_text(json.dumps({'lanes': {'G4': {'prompt_sets': {'default': {'generated_input': 'prompts'}},
                                                   'server_flags': ['SYNTHETIC_FIXED_FLAGS']}}}))
        self.manifest = {'candidate_sha': 'a' * 40, 'tokenizer': recipe['tokenizer'],
            'lane': 'G4', 'lane_manifest': runtime.pin(lane), 'build_receipt': runtime.pin(receipt),
            'cases': {'BC-GPU-003': {'command': ['synthetic-no-launch', '--manifest', str(lane)], 'out': str(self.root / 'attempt')}},
            'generated_inputs': [{'recipe': recipe, 'structural': generated.validate(recipe, receipt, 'a' * 40)}]}
        self.manifest['inputs'] = [self.manifest['build_receipt'], self.manifest['lane_manifest']] + [recipe[key] for key in ('generator', 'dataset', 'tokenizer', 'template')]
        self.parent = self.root / 'parent.json'
        self.parent.write_text(json.dumps(self.manifest))

    def generate(self, command, directory):
        (directory / 'prompts.jsonl').write_text('{"SYNTHETIC_INPUT":true}\n')
        (directory / 'selection-report.json').write_text('{"SYNTHETIC_DIAGNOSTIC":true}\n')
        (directory / 'generator.log').write_text('SYNTHETIC LOG; NO GENERATOR EXECUTED\n')

    def prepare(self):
        with patch.object(generated, 'run_generator', side_effect=self.generate):
            return runtime.prepare(self.parent, self.manifest, self.root / 'runtime')

    def test_marked_frozen_input_rejected_before_generation_or_output(self):
        smoke = self.root / 'smoke'; smoke.mkdir()
        (smoke / result_writer.SMOKE_MARKER).write_text(json.dumps(
            {'classification': result_writer.SMOKE_CLASS, 'expectations_sha256': 'f' * 64}))
        recipe = self.manifest['generated_inputs'][0]['recipe']
        dataset = smoke / 'dataset.jsonl'
        dataset.write_bytes(Path(recipe['dataset']['path']).read_bytes())
        old = recipe['dataset']
        recipe['dataset'] = runtime.pin(dataset)
        entry = self.manifest['generated_inputs'][0]
        entry['structural'] = generated.validate(recipe, self.manifest['build_receipt']['path'], 'a' * 40)
        self.manifest['inputs'] = [recipe['dataset'] if x == old else x for x in self.manifest['inputs']]
        self.parent.write_text(json.dumps(self.manifest))
        with patch.object(generated, 'run_generator') as generator:
            with self.assertRaises(result_writer.NonCertificationEvidence):
                runtime.prepare(self.parent, self.manifest, self.root / 'runtime')
            generator.assert_not_called()
        self.assertFalse((self.root / 'runtime').exists())

    def test_runtime_binding_seed_rejects_boolean_and_float_equal_to_integer(self):
        entry = self.manifest['generated_inputs'][0]
        entry['recipe']['seed'] = 1
        entry['structural'] = generated.validate(entry['recipe'], self.manifest['build_receipt']['path'], 'a' * 40)
        self.parent.write_text(json.dumps(self.manifest))
        _, bound = self.prepare()
        item = bound['generated_bindings']['prompts']
        path = Path(item['path'])
        binding = json.loads(path.read_text())
        for value in (True, 1.0):
            with self.subTest(seed=value):
                binding['seed'] = value
                path.write_text(json.dumps(binding))
                item.update(runtime.pin(path))
                with self.assertRaisesRegex(ValueError, 'source/config/seed'):
                    runtime.verify(bound)

    def test_runtime_envelope_binds_actual_bytes_and_preserves_parent(self):
        original = self.parent.read_bytes()
        path, bound = self.prepare()
        runtime.verify(bound)
        self.assertEqual(generated.preflight_label(bound), 'PREFLIGHT-OK')
        self.assertEqual(self.parent.read_bytes(), original)
        self.assertEqual(bound['parent_structural_manifest'], runtime.pin(self.parent))
        command = bound['cases']['BC-GPU-003']['command']
        self.assertEqual(command[command.index('--manifest') + 1], bound['lane_manifest']['path'])
        lane = json.loads(Path(bound['lane_manifest']['path']).read_text())
        actual = lane['lanes']['G4']['prompt_sets']['default']
        self.assertEqual(actual, runtime.pin(self.root / 'runtime/datasets/default.jsonl'))
        self.assertEqual(json.loads(path.read_text()), bound)

    def test_report_binds_actual_hashes_and_structural_only_cannot_pass(self):
        import execution_evidence
        path, bound = self.prepare()
        out = self.root / 'report'
        out.mkdir()
        record = {'ID': 'BC-GPU-003', 'candidate_sha': 'a' * 40, 'result': 'PASS', 'evidence_paths': []}
        env = {'CERT_INPUT_MANIFEST': str(path), 'CERT_CASE_ID': 'BC-GPU-003'}
        with patch.dict(execution_evidence.os.environ, env, clear=True):
            enriched = execution_evidence.enrich(out, record)
        ledger = enriched['versions']['generated_inputs']['prompts']
        self.assertEqual(ledger['input_sha256'], runtime.pin(self.root / 'runtime/datasets/default.jsonl')['sha256'])
        self.assertEqual(ledger['input_sha256'], ledger['rederived_sha256'])
        env['CERT_INPUT_MANIFEST'] = str(self.parent)
        with patch.dict(execution_evidence.os.environ, env, clear=True), self.assertRaisesRegex(ValueError, 'structural-only'):
            execution_evidence.enrich(out, record)

    def test_binding_output_parent_or_server_mutation_fails(self):
        _, bound = self.prepare()
        parent_bytes = self.parent.read_bytes()
        self.parent.write_bytes(parent_bytes + b' ')
        with self.assertRaises(ValueError):
            runtime.verify(bound)
        self.parent.write_bytes(parent_bytes)
        path = Path(bound['lane_manifest']['path'])
        lane = json.loads(path.read_text())
        lane['lanes']['G4']['server_flags'] = ['changed']
        path.write_text(json.dumps(lane))
        changed = copy.deepcopy(bound)
        changed['lane_manifest'] = runtime.pin(path)
        # Even updating the pin cannot authorize changed server configuration.
        with self.assertRaisesRegex(ValueError, 'server configuration'):
            runtime.verify(changed)

    def test_nondeterministic_generation_never_emits_runtime_case(self):
        def changing(command, directory):
            self.generate(command, directory)
            (directory / 'prompts.jsonl').write_text(directory.name)
        with patch.object(generated, 'run_generator', side_effect=changing), self.assertRaisesRegex(ValueError, 're-derivation mismatch'):
            runtime.prepare(self.parent, self.manifest, self.root / 'runtime')
        self.assertFalse((self.root / 'runtime/case.json').exists())
        self.assertFalse((self.root / 'runtime/prompts/binding.json').exists())
        self.assertTrue((self.root / 'runtime/prompts/actual/prompts.jsonl').exists())


if __name__ == '__main__':
    unittest.main()
