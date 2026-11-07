# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic source/artifact/probe/launch fixtures; never real certification."""
import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch, Mock
import case_launcher
import freeze_case
import generated_inputs as generated
import runtime_inputs
from build_role_test_fixture import bind
from execution_evidence import digest


class FreezeGeneratedTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        def file(name, data):
            path = self.root / name
            path.write_text(data)
            return path
        self.binary = file('shipped', '#!/bin/sh\n# SYNTHETIC SHIPPED\nexit 0\n')
        self.generator = file('prompts', '#!/bin/sh\n# SYNTHETIC CPU GENERATOR\nexit 0\n')
        for path in (self.binary, self.generator): path.chmod(0o755)
        fixture = bind({'candidate_sha': 'a' * 40, 'candidate': {'path': str(self.binary), 'sha256': digest(self.binary)}},
                       self.root, additional={'metrum-ai-bench-cli-prompts': self.generator})
        registry = json.loads(Path(fixture['build_receipt']['path']).read_text())['builds']['measured']['binaries']
        tokenizer = file('tokenizer.json', '{"SYNTHETIC":true}')
        recipe = {'id': 'prompts', 'kind': generated.KIND, 'generator': registry['metrum-ai-bench-cli-prompts'],
                  'generator_version': '1.8.1', 'seed': 42, 'config': {'count': 4, 'profile': 'chat-short'},
                  'dataset': runtime_inputs.pin(file('dataset.jsonl', 'SYNTHETIC DATASET')),
                  'template': runtime_inputs.pin(file('template.txt', 'SYNTHETIC TEMPLATE')),
                  'tokenizer': {**runtime_inputs.pin(tokenizer), 'revision': 'b' * 40}}
        recipes = file('recipes.json', json.dumps({'recipes': [recipe]}))
        sut = file('sut.json', json.dumps({'host_os': 'Linux', 'memory_gb': 128, 'gpu': {'memory_gb': 48}}))
        catalog = file('catalog.csv', 'ID,Tier,Parallel group\nBC-GPU-011,RC-GPU,G4\n')
        selection = file('selection.json', json.dumps({'candidate_sha': 'a' * 40, 'catalog_sha256': digest(catalog), 'selected': {'BC-GPU-011': ['core']}}))
        config = {'image': 'synthetic/image@sha256:' + '0' * 64, 'model': 'Qwen/Qwen3-8B',
                  'revision': 'b' * 40, 'port': 19001, 'proxy_port': 19002,
                  'server_flags': ['--reasoning-parser', 'qwen3', '--no-enable-prefix-caching'],
                  'owner': 'fixture', 'evidence_link': 'fixture', 'tokenizer': runtime_inputs.pin(tokenizer),
                  'prompt_sets': {'default': {'generated_input': 'prompts'}}}
        lane = file('lane.json', json.dumps({'candidate_sha': 'a' * 40, 'candidate_binary_sha256': digest(self.binary),
                                           'baseline_binary_sha256': digest(self.binary), 'lanes': {'G4': config}}))
        self.args = SimpleNamespace(build_receipt=Path(fixture['build_receipt']['path']), selection=selection, catalog=catalog,
            case='BC-GPU-011', lane_manifest=lane, candidate=self.binary, baseline=self.binary, sut=sut,
            tokenizer=tokenizer, tokenizer_revision='b' * 40, all_smi=self.binary, input=[], command_json=None,
            generated_recipes=recipes, out=self.root / 'frozen.json', run_out=self.root / 'attempt')
        probe = patch('tokenizer_probe.probe')
        probe.start()
        self.addCleanup(probe.stop)

    def generate(self, command, directory):
        (directory / 'prompts.jsonl').write_text('{"SYNTHETIC_PROMPT":true}\n')
        (directory / 'selection-report.json').write_text('{"SYNTHETIC_REPORT":true}\n')
        (directory / 'generator.log').write_text('SYNTHETIC; NO GENERATOR EXECUTED\n')

    def test_freezer_structural_then_original_concrete_preflight(self):
        import selected_set
        frozen = freeze_case.freeze(self.args)
        self.assertEqual(generated.preflight_label(frozen), 'STRUCTURAL-PREFLIGHT-OK')
        index = self.root / 'index.json'
        index.write_text(json.dumps({'candidate_sha': 'a' * 40, 'selection_sha256': digest(self.args.selection),
            'catalog_sha256': digest(self.args.catalog), 'manifests': [runtime_inputs.pin(self.args.out)]}))
        cli = ['selected_set.py', '--selection', str(self.args.selection), '--catalog', str(self.args.catalog),
               '--index', str(index), '--preflight']
        output = io.StringIO()
        with patch('sys.argv', cli), contextlib.redirect_stdout(output):
            selected_set.main()
        self.assertEqual(output.getvalue().strip(), 'STRUCTURAL-PREFLIGHT-OK BC-GPU-011')
        with patch.object(generated, 'run_generator', side_effect=self.generate):
            _, bound = runtime_inputs.prepare(self.args.out, frozen, self.root / 'runtime')
        self.assertEqual(case_launcher.preflight(bound), ['BC-GPU-011'])
        self.assertEqual(generated.preflight_label(bound), 'PREFLIGHT-OK')

    def test_launch_binds_runtime_before_child_and_keeps_parent(self):
        frozen = freeze_case.freeze(self.args)
        original = self.args.out.read_bytes()
        def launch(command, **kwargs):
            runtime_path = Path(kwargs['env']['CERT_INPUT_MANIFEST'])
            self.assertNotEqual(runtime_path, self.args.out)
            bound = json.loads(runtime_path.read_text())
            runtime_inputs.verify(bound)
            self.assertEqual(generated.preflight_label(bound), 'PREFLIGHT-OK')
            target = self.args.run_out / 'results/BC-GPU-011.json'
            target.parent.mkdir(parents=True)
            target.write_text('{"SYNTHETIC_RECORD_PLACEHOLDER":true}')
            return Mock(wait=Mock(return_value=0), poll=Mock(return_value=0))
        record = {'ID': 'BC-GPU-011', 'candidate_sha': 'a' * 40, 'result': 'PASS'}
        with patch.object(generated, 'run_generator', side_effect=self.generate), \
             patch.object(case_launcher.subprocess, 'run'), \
             patch.object(case_launcher.subprocess, 'Popen', side_effect=launch) as start, \
             patch('execution_evidence.report_rows', return_value=[record]):
            case_launcher.run_case(self.args.out, frozen, 'BC-GPU-011')
        self.assertEqual(start.call_count, 1)
        self.assertEqual(self.args.out.read_bytes(), original)

    def test_mismatch_stops_child_and_retains_typed_failure(self):
        frozen = freeze_case.freeze(self.args)
        def changing(command, directory):
            self.generate(command, directory)
            (directory / 'prompts.jsonl').write_text(directory.name)
        with patch.object(generated, 'run_generator', side_effect=changing), \
             patch.object(case_launcher.subprocess, 'Popen') as start, self.assertRaisesRegex(ValueError, 're-derivation mismatch'):
            case_launcher.run_case(self.args.out, frozen, 'BC-GPU-011')
        start.assert_not_called()
        record = json.loads((self.args.run_out / 'results/BC-GPU-011.json').read_text())
        self.assertEqual(record['result'], 'FAIL')
        self.assertEqual(record['infra_history'][0]['type'], 'infra_failed')
        self.assertIsNone(record['observed_values']['request_count'])
        self.assertTrue((self.root / 'attempt-lifecycle/generated/prompts/actual/prompts.jsonl').exists())


if __name__ == '__main__':
    unittest.main()
