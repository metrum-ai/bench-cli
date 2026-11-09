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
import r4_model_pins as pins
from build_role_test_fixture import bind
from execution_evidence import digest


class FreezeGeneratedTests(unittest.TestCase):
    def setUp(self):
        verifier = patch('harness_integrity.verify', return_value=[])
        verifier.start()
        self.addCleanup(verifier.stop)
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        def file(name, data):
            path = self.root / name
            path.write_text(data)
            return path
        self.binary = file('shipped', '#!/bin/sh\n# SYNTHETIC SHIPPED\nexit 0\n')
        self.generator = file('prompts', '#!/bin/sh\n# SYNTHETIC CPU GENERATOR\nexit 0\n')
        strategic = file('metrum-ai-bench-cli-strategic', '#!/bin/sh\n# SYNTHETIC STRATEGIC\nexit 0\n')
        vlm = file('metrum-ai-bench-cli-vlm', '#!/bin/sh\n# SYNTHETIC VLM\nexit 0\n')
        for path in (self.binary, self.generator, strategic, vlm): path.chmod(0o755)
        fixture = bind({'candidate_sha': 'a' * 40, 'candidate': {'path': str(self.binary), 'sha256': digest(self.binary)}},
                       self.root, additional={'metrum-ai-bench-cli-prompts': self.generator,
                                              'metrum-ai-bench-cli-strategic': strategic,
                                              'metrum-ai-bench-cli-vlm': vlm})
        registry = json.loads(Path(fixture['build_receipt']['path']).read_text())['builds']['measured']['binaries']
        tokenizer = file('tokenizer.json', '{"SYNTHETIC":true}')
        recipe = {'id': 'prompts', 'kind': generated.KIND, 'generator': registry['metrum-ai-bench-cli-prompts'],
                  'generator_version': '1.8.1', 'seed': 42, 'config': {'count': 4, 'profile': 'chat-short'},
                  'dataset': runtime_inputs.pin(file('dataset.jsonl', 'SYNTHETIC DATASET')),
                  'template': runtime_inputs.pin(file('template.txt', 'SYNTHETIC TEMPLATE')),
                  'tokenizer': {**runtime_inputs.pin(tokenizer), 'revision': pins.GENERAL_REVISION}}
        recipes = file('recipes.json', json.dumps({'recipes': [recipe]}))
        sut = file('sut.json', json.dumps({'host_os': 'Linux', 'memory_gb': 128, 'gpu': {'memory_gb': 48}}))
        import select_cases
        import validate_results
        catalog = file('catalog.csv', 'ID,Tier,Test case,Selection,Area,Parallel group\nBC-GPU-011,RC-GPU,fixture,core,GPU,G4\n')
        selection = file('selection.json', json.dumps(select_cases.make_selection(
            validate_results.load_catalog(catalog), digest(catalog), 'a' * 40, 0.1, 3, [], [])))
        image = file('synthetic-image.png', 'SYNTHETIC IMAGE')
        vlm_prompt = file('vlm-prompt.jsonl', json.dumps({'prompt': 'fixture', 'image_url': str(image)}) + '\n')
        config = {'image': pins.IMAGE, 'model': pins.GENERAL_MODEL,
                  'revision': pins.GENERAL_REVISION, 'port': 19001, 'proxy_port': 19002,
                  'server_flags': ['--reasoning-parser', 'qwen3', '--no-enable-prefix-caching'],
                  'owner': 'fixture', 'evidence_link': 'fixture', 'tokenizer': runtime_inputs.pin(tokenizer),
                  'strategic_binary': registry['metrum-ai-bench-cli-strategic'],
                  'vlm_binary': registry['metrum-ai-bench-cli-vlm'], 'vlm_image': runtime_inputs.pin(image),
                  'vlm': {'model': pins.GENERAL_MODEL, 'revision': pins.GENERAL_REVISION,
                          'image': pins.IMAGE,
                          'server_flags': ['--reasoning-parser', 'qwen3', '--no-enable-prefix-caching']},
                  'prompt_sets': {'default': {'generated_input': 'prompts'}, 'vlm': runtime_inputs.pin(vlm_prompt)}}
        lane = file('lane.json', json.dumps({'candidate_sha': 'a' * 40, 'candidate_binary_sha256': digest(self.binary),
                                           'baseline_binary_sha256': digest(self.binary), 'lanes': {'G4': config}}))
        self.args = SimpleNamespace(build_receipt=Path(fixture['build_receipt']['path']), tool_manifest=Path(fixture['tool_manifest']['path']), selection=selection, catalog=catalog,
            case='BC-GPU-011', lane_manifest=lane, candidate=self.binary, baseline=self.binary, sut=sut,
            tokenizer=tokenizer, tokenizer_revision=pins.GENERAL_REVISION, all_smi=self.binary, input=[], command_json=None,
            generated_recipes=recipes, out=self.root / 'frozen.json', run_out=self.root / 'attempt')
        probe = patch('tokenizer_probe.probe')
        probe.start()
        self.addCleanup(probe.stop)

    def generate(self, command, directory, tool_dir=None):
        (directory / 'prompts.jsonl').write_text('{"SYNTHETIC_PROMPT":true}\n')
        (directory / 'selection-report.json').write_text('{"SYNTHETIC_REPORT":true}\n')
        (directory / 'generator.log').write_text('SYNTHETIC; NO GENERATOR EXECUTED\n')

    def test_missing_case_specific_input_fails_before_freeze(self):
        lane = json.loads(self.args.lane_manifest.read_text())
        del lane['lanes']['G4']['strategic_binary']
        self.args.lane_manifest.write_text(json.dumps(lane))
        with self.assertRaises(KeyError):
            freeze_case.freeze(self.args)
        self.assertFalse(self.args.out.exists())

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

    def test_preflight_revalidates_r4_lane_and_oci_descriptors(self):
        import copy
        frozen = freeze_case.freeze(self.args)
        missing = copy.deepcopy(frozen)
        del missing['r4_oci']
        with self.assertRaisesRegex(ValueError, 'OCI.*required'):
            case_launcher.preflight(missing)
        bad = copy.deepcopy(frozen)
        bad['r4_oci']['amd64']['sha256'] = '0' * 64
        with self.assertRaisesRegex(ValueError, 'hash mismatch'):
            case_launcher.preflight(bad)
        lane = json.loads(self.args.lane_manifest.read_text())
        lane['lanes']['G4']['revision'] = 'f' * 40
        self.args.lane_manifest.write_text(json.dumps(lane))
        bad = copy.deepcopy(frozen)
        bad['lane_manifest']['sha256'] = digest(self.args.lane_manifest)
        with self.assertRaisesRegex(ValueError, 'r4 model'):
            case_launcher.preflight(bad)

    def test_launch_binds_runtime_before_child_and_keeps_parent(self):
        frozen = freeze_case.freeze(self.args)
        original = self.args.out.read_bytes()
        def launch(command, **kwargs):
            self.assertEqual(kwargs['env'], {})
            self.assertEqual(command[1], '-i')
            assignments = dict(item.split('=', 1) for item in command[2:] if '=' in item and not item.startswith('/'))
            runtime_path = Path(assignments['CERT_INPUT_MANIFEST'])
            self.assertNotEqual(runtime_path, self.args.out)
            bound = json.loads(runtime_path.read_text())
            runtime_inputs.verify(bound)
            record['input_hashes'] = {'execution_manifest': digest(runtime_path)}
            record['versions']['execution_tools'] = bound['tools']
            self.assertEqual(generated.preflight_label(bound), 'PREFLIGHT-OK')
            target = self.args.run_out / 'results/BC-GPU-011.json'
            target.parent.mkdir(parents=True)
            target.write_text('{"SYNTHETIC_RECORD_PLACEHOLDER":true}')
            return Mock(wait=Mock(return_value=0), poll=Mock(return_value=0))
        import r4_image_provenance as provenance
        record = {'ID': 'BC-GPU-011', 'candidate_sha': 'a' * 40, 'result': 'PASS',
                  'versions': {'server_image': pins.IMAGE, 'model': pins.GENERAL_MODEL,
                               'model_revision': pins.GENERAL_REVISION, 'image_index': provenance.INDEX,
                               'platform_manifest': provenance.AMD64, 'image_config': provenance.CONFIG,
                               'image_platform': 'linux/amd64'}}
        with patch.object(generated, 'run_generator', side_effect=self.generate), \
             patch.object(case_launcher.subprocess, 'run'), \
             patch.object(case_launcher.subprocess, 'Popen', side_effect=launch) as start, \
             patch('execution_evidence.report_rows', return_value=[record]):
            case_launcher.run_case(self.args.out, frozen, 'BC-GPU-011')
        self.assertEqual(start.call_count, 1)
        self.assertEqual(self.args.out.read_bytes(), original)

    def test_child_pass_without_execution_binding_writes_lane_error(self):
        frozen = freeze_case.freeze(self.args)
        def launch(command, **kwargs):
            target = self.args.run_out / 'results/BC-GPU-011.json'
            target.parent.mkdir(parents=True)
            target.write_text('{"SYNTHETIC_RECORD_PLACEHOLDER":true}')
            return Mock(wait=Mock(return_value=0), poll=Mock(return_value=0))
        record = {'ID': 'BC-GPU-011', 'candidate_sha': 'a' * 40, 'result': 'PASS'}
        with patch.object(generated, 'run_generator', side_effect=self.generate), \
             patch.object(case_launcher.subprocess, 'run'), \
             patch.object(case_launcher.subprocess, 'Popen', side_effect=launch), \
             patch('execution_evidence.report_rows', return_value=[record]), \
             self.assertRaisesRegex(ValueError, 'executed manifest'):
            case_launcher.run_case(self.args.out, frozen, 'BC-GPU-011')
        self.assertTrue((self.args.run_out / 'results/lane-error.json').is_file())

    def test_orphan_runtime_entries_reject_preflight_label_and_direct_launch(self):
        import copy
        frozen = freeze_case.freeze(self.args)
        for key in ('generated_bindings', 'parent_structural_manifest', 'generated_inputs'):
            orphan = copy.deepcopy(frozen)
            del orphan['generated_inputs']
            orphan[key] = None if key == 'generated_inputs' else {}
            with self.subTest(field=key), patch.object(case_launcher.subprocess, 'Popen') as child:
                with self.assertRaisesRegex(ValueError, 'structural entries'):
                    case_launcher.preflight(orphan)
                with self.assertRaisesRegex(ValueError, 'structural entries'):
                    generated.preflight_label(orphan)
                child.assert_not_called()
        orphan = copy.deepcopy(frozen)
        del orphan['generated_inputs']
        orphan['generated_bindings'] = {}
        path = self.root / 'orphan-case.json'
        path.write_text(json.dumps(orphan))
        with patch.object(case_launcher.subprocess, 'Popen') as child, \
             self.assertRaisesRegex(ValueError, 'structural entries'):
            case_launcher.run_case(path, orphan, self.args.case)
        child.assert_not_called()
        record = json.loads((self.args.run_out / 'results' / (self.args.case + '.json')).read_text())
        self.assertEqual(record['result'], 'FAIL')
        self.assertEqual(record['infra_history'][0]['type'], 'infra_failed')
        self.assertIsNone(record['observed_values']['request_count'])
        self.assertIsNone(record['observed_values']['error_count'])
        self.assertTrue(any('structural entries' in item for item in record['input_integrity_errors']))
        import execution_evidence
        env = {'CERT_INPUT_MANIFEST': str(path), 'CERT_CASE_ID': self.args.case}
        with patch.dict(execution_evidence.os.environ, env, clear=True), \
             self.assertRaisesRegex(ValueError, 'structural entries'):
            execution_evidence.enrich(self.args.run_out, {**record, 'result': 'PASS'})

    def test_freezer_pins_compiled_harness_and_rejects_source_drift(self):
        module_root = Path(freeze_case.__file__).parent
        repo = module_root.parents[2]
        sources = freeze_case.harness_sources(module_root)
        for relative in ('xtask/src/local_smoke.rs', 'xtask/src/main.rs', 'xtask/Cargo.toml',
                         'Cargo.toml', 'Cargo.lock', '.cargo/config.toml', 'src/count_points.rs'):
            self.assertIn((repo / relative).resolve(), sources)
        # Mutate a synthetic source supplied by the source-inventory seam, never
        # the live repository or another test's source tree.
        synthetic = self.root / 'synthetic-local-smoke.rs'
        synthetic.write_text('// SYNTHETIC ORIGINAL HARNESS\n')
        with patch.object(freeze_case, 'harness_sources', return_value=sources | {synthetic}):
            frozen = freeze_case.freeze(self.args)
        frozen_paths = {item['path'] for item in frozen['inputs']}
        self.assertIn(str((repo / 'xtask/src/local_smoke.rs').resolve()), frozen_paths)
        self.assertIn(str(synthetic), frozen_paths)
        synthetic.write_text('// SYNTHETIC CHANGED HARNESS\n')
        # Version probing is CPU-only, not measured workload issuance.
        with patch.object(case_launcher.subprocess, 'run'), \
             patch.object(case_launcher.subprocess, 'Popen') as child, \
             self.assertRaisesRegex(ValueError, 'pinned input hash mismatch'):
            case_launcher.preflight(frozen)
        child.assert_not_called()

    def test_l1_freezer_requires_template_and_freezes_internal_selector_seed_separately(self):
        # No generator runs; adapt synthetic workflow paths to L1's real layout contract.
        receipt_path = self.args.build_receipt
        receipt = json.loads(receipt_path.read_text())
        registry = receipt['builds']['measured']['binaries']
        for name, item in registry.items():
            path = self.root / name
            path.write_bytes(Path(item['path']).read_bytes())
            path.chmod(0o755)
            item['path'] = str(path)
        receipt_path.write_text(json.dumps(receipt))
        self.args.candidate = Path(registry['metrum-ai-bench-cli-llm']['path'])
        recipes = json.loads(self.args.generated_recipes.read_text())['recipes']
        recipe = recipes[0]
        self.args.generated_recipes = None
        sut = json.loads(self.args.sut.read_text())
        sut['model'] = {'id': pins.GENERAL_MODEL, 'revision': pins.GENERAL_REVISION}
        self.args.sut.write_text(json.dumps(sut))
        lane = json.loads(self.args.lane_manifest.read_text())
        cfg = lane['lanes']['G4']
        cfg.update(prompt_sets={'default': recipe['dataset']}, sut=runtime_inputs.pin(self.args.sut),
                   prompt_binary=registry['metrum-ai-bench-cli-prompts'])
        lane['lanes'] = {'L1': cfg}
        self.args.case = 'BC-LIVE-001'
        self.args.catalog.write_text('ID,Tier,Parallel group\nBC-LIVE-001,RC-GPU,L1\n')
        self.args.selection.write_text(json.dumps({'candidate_sha': 'a' * 40, 'catalog_sha256': digest(self.args.catalog),
                                                  'selected': {'BC-LIVE-001': ['core']}}))
        self.args.lane_manifest.write_text(json.dumps(lane))
        with self.assertRaisesRegex(ValueError, 'file pin'):
            freeze_case.freeze(self.args)
        cfg['template'] = recipe['template']
        self.args.lane_manifest.write_text(json.dumps(lane))
        frozen = freeze_case.freeze(self.args)
        self.assertEqual(generated.preflight_label(frozen), 'STRUCTURAL-PREFLIGHT-OK')
        self.assertEqual(frozen['derived_inputs']['contract']['seed'], 7)
        self.assertEqual(frozen['derived_inputs']['contract']['config'], {'count': 16, 'profile': 'chat-short'})
        self.assertEqual(frozen['derived_inputs']['contract']['template'], recipe['template'])

    def test_mismatch_stops_child_and_retains_typed_failure(self):
        frozen = freeze_case.freeze(self.args)
        def changing(command, directory, tool_dir=None):
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
