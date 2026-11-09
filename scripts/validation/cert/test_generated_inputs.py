# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic structural-contract unit tests; no generator or request is executed."""
import copy
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
import generated_inputs as generated
from execution_evidence import digest


class GeneratedInputTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.sha = 'a' * 40
        def pin(name):
            path = self.root / name
            path.write_text('SYNTHETIC STRUCTURAL INPUT ' + name)
            return {'path': str(path), 'sha256': digest(path)}
        self.recipe = {'id': 'short-prompts', 'kind': generated.KIND, 'generator': pin('generator'),
                       'generator_version': '1.8.1', 'seed': 42, 'config': {'count': 4, 'profile': 'chat-short'},
                       'dataset': pin('dataset.jsonl'), 'template': pin('template.txt'),
                       'tokenizer': {**pin('tokenizer.json'), 'revision': 'b' * 40}}
        self.receipt = self.root / 'receipt.json'
        self.receipt.write_text(json.dumps({'version': '1.8.1', 'builds': {'measured': {
            'binaries': {'metrum-ai-bench-cli-prompts': self.recipe['generator']}}}}))
        self.receipt_guard = patch.object(generated.build_roles, 'load_receipt')
        self.guard = self.receipt_guard.start()
        self.addCleanup(self.receipt_guard.stop)

    def validate(self, recipe=None):
        return generated.validate(self.recipe if recipe is None else recipe, self.receipt, self.sha)

    def test_structural_status_never_invents_output_hash_and_checks_receipt(self):
        result = self.validate()
        self.assertEqual(result, {'stage': 'structural-only', 'recipe_sha256': generated.contract_hash(self.recipe)})
        self.guard.assert_called_once_with(self.receipt, self.sha)
        self.assertEqual({path.name for path in self.root.iterdir()},
                         {'generator', 'dataset.jsonl', 'template.txt', 'tokenizer.json', 'receipt.json'})
        command = generated.argv(self.recipe, self.root / 'future-runtime')
        self.assertIn('--local-jsonl', command)
        self.assertNotIn('--url', command)
        self.assertEqual(command[command.index('--seed') + 1], '42')
        self.assertFalse((self.root / 'future-runtime').exists())

    def test_future_hash_unknown_kind_wrong_generator_and_version_fail(self):
        for key, value in [('output_sha256', '0' * 64), ('kind', 'arbitrary-command'),
                           ('generator', self.recipe['dataset']), ('generator_version', 'other')]:
            bad = copy.deepcopy(self.recipe)
            bad[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.validate(bad)

    def test_unknown_flags_invalid_seed_or_config_fail_before_execution(self):
        for config in ({'count': 4, 'url': 'example', 'profile': 'chat-short'}, {'count': True, 'profile': 'chat-short'},
                       {'count': 4}, {'count': 4, 'profile': []}, {'count': 4, 'profile': 'chat-short', 'isl_target': 8},
                       {'count': 4, 'isl_target': float('nan'), 'osl_target': 8}):
            bad = copy.deepcopy(self.recipe)
            bad['config'] = config
            with self.subTest(config=config), self.assertRaises(ValueError):
                self.validate(bad)
        for seed in (True, -1, 2**64, '42'):
            bad = copy.deepcopy(self.recipe)
            bad['seed'] = seed
            with self.subTest(seed=seed), self.assertRaises(ValueError):
                self.validate(bad)

    def test_tokenizer_template_dataset_and_generator_drift_fail(self):
        for field in ('tokenizer', 'template', 'dataset', 'generator'):
            path = Path(self.recipe[field]['path'])
            original = path.read_bytes()
            path.write_bytes(original + b'changed')
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.validate()
            path.write_bytes(original)

    def test_structural_manifest_freezes_contract_and_dependency_pins_but_cannot_launch(self):
        manifest = {'candidate_sha': self.sha, 'cases': {'BC-GPU-003': {}},
                    'build_receipt': {'path': str(self.receipt), 'sha256': digest(self.receipt)},
                    'tokenizer': self.recipe['tokenizer'],
                    'generated_inputs': [{'recipe': self.recipe, 'structural': self.validate()}]}
        manifest['inputs'] = [manifest['build_receipt']] + [self.recipe[key] for key in ('generator', 'dataset', 'tokenizer', 'template')]
        self.assertEqual(generated.structural_context(manifest), {self.recipe['id']: self.recipe})
        self.assertEqual(generated.preflight_label(manifest), 'STRUCTURAL-PREFLIGHT-OK')
        self.assertEqual(generated.preflight_label({}), 'PREFLIGHT-OK')
        import case_launcher
        manifest['candidate'] = self.recipe['generator']  # Mocked role guard; no executable runs.
        from build_role_test_fixture import tool_fixture
        manifest['tools'], tool_path = tool_fixture(self.root)
        manifest['tool_manifest'] = {'path': str(tool_path), 'sha256': digest(tool_path)}
        manifest['lane'] = 'G1'
        launcher = Path('/bin/true').resolve()
        manifest['cases']['BC-GPU-003'] = {'out': str(self.root / 'attempt'),
            'launcher': {'path': str(launcher), 'sha256': digest(launcher)}, 'command': [str(launcher)]}
        with patch.object(case_launcher.build_roles, 'check_manifest'), \
             patch('runtime_inputs.prepare', side_effect=ValueError('runtime binding incomplete')), \
             patch('result_writer.write_record') as write, \
             patch.object(case_launcher.subprocess, 'Popen') as start, self.assertRaisesRegex(ValueError, 'runtime binding incomplete'):
            case_launcher.run_case(self.root / 'not-executed.json', manifest, 'BC-GPU-003')
        start.assert_not_called()
        self.assertEqual(write.call_args.args[1]['infra_history'][0]['type'], 'infra_failed')
        bad = copy.deepcopy(manifest)
        bad['generated_inputs'][0]['recipe']['seed'] += 1
        with self.assertRaisesRegex(ValueError, 'frozen generator contract'):
            generated.structural_context(bad)
        bad = copy.deepcopy(manifest)
        bad['inputs'].pop()
        with self.assertRaisesRegex(ValueError, 'freeze every generator dependency'):
            generated.structural_context(bad)

    def fake_generator(self, command, directory):
        self.assertIn('--local-jsonl', command)
        (directory / 'prompts.jsonl').write_text('{"SYNTHETIC_TEST_PROMPT":true}\n')
        (directory / 'selection-report.json').write_text('{"SYNTHETIC_TEST_REPORT":true}\n')

    def test_runtime_binds_actual_bytes_only_after_independent_rederivation(self):
        expected = self.validate()['recipe_sha256']
        root = self.root / 'runtime'
        with patch.object(generated, 'run_generator', side_effect=self.fake_generator) as run:
            binding = generated.materialize(self.recipe, self.receipt, self.sha, root, expected)
        self.assertEqual(run.call_count, 2)
        self.assertEqual(binding['stage'], 'runtime-bound')
        self.assertEqual(binding['actual']['prompts.jsonl']['sha256'], digest(root / 'actual/prompts.jsonl'))
        self.assertEqual(binding['actual']['prompts.jsonl']['sha256'], binding['rederived']['prompts.jsonl']['sha256'])
        self.assertEqual(json.loads((root / 'binding.json').read_text()), binding)
        with self.assertRaises(FileExistsError):
            generated.materialize(self.recipe, self.receipt, self.sha, root, expected)

    def test_runtime_seed_drift_blocks_before_generator_or_output_creation(self):
        expected = self.validate()['recipe_sha256']
        changed = copy.deepcopy(self.recipe)
        changed['seed'] += 1
        root = self.root / 'runtime'
        with patch.object(generated, 'run_generator') as run, self.assertRaisesRegex(ValueError, 'drift'):
            generated.materialize(changed, self.receipt, self.sha, root, expected)
        run.assert_not_called()
        self.assertFalse(root.exists())

    def test_runtime_nondeterminism_preserves_both_outputs_but_no_binding(self):
        def nondeterministic(command, directory):
            self.fake_generator(command, directory)
            (directory / 'prompts.jsonl').write_text('SYNTHETIC ' + directory.name)
        expected = self.validate()['recipe_sha256']
        root = self.root / 'runtime'
        with patch.object(generated, 'run_generator', side_effect=nondeterministic), self.assertRaisesRegex(ValueError, 're-derivation mismatch'):
            generated.materialize(self.recipe, self.receipt, self.sha, root, expected)
        self.assertTrue((root / 'actual/prompts.jsonl').is_file())
        self.assertTrue((root / 'rederived/prompts.jsonl').is_file())
        self.assertFalse((root / 'binding.json').exists())

    def test_generator_has_owned_group_and_no_ambient_credentials(self):
        directory = self.root / 'run'
        directory.mkdir()
        with patch.dict(generated.os.environ, {'HF_TOKEN': 'SYNTHETIC_NOT_A_CREDENTIAL', 'HTTPS_PROXY': 'SYNTHETIC'}, clear=True), \
             patch.object(generated.subprocess, 'Popen') as start:
            start.return_value.wait.return_value = 0
            start.return_value.poll.return_value = 0
            generated.run_generator(['synthetic-no-execution'], directory)
        self.assertTrue(start.call_args.kwargs['start_new_session'])
        self.assertEqual(set(start.call_args.kwargs['env']),
                         {'PATH', 'LANG', 'LC_ALL', 'HF_HUB_OFFLINE', 'TRANSFORMERS_OFFLINE', 'NO_BANNER'})

    def test_seed_and_config_changes_change_structural_contract_not_future_digest(self):
        original = self.validate()['recipe_sha256']
        for key, value in [('seed', 43), ('config', {'count': 5, 'profile': 'chat-short'})]:
            changed = copy.deepcopy(self.recipe)
            changed[key] = value
            self.assertNotEqual(self.validate(changed)['recipe_sha256'], original)


if __name__ == '__main__':
    unittest.main()
