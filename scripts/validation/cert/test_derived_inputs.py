# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic internal-L1 selection boundary; no real generator or model requests."""
import copy
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
import derived_inputs as derived
import generated_inputs as generated
from runtime_inputs import pin


class DerivedInputTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        def asset(name):
            path = self.root / name
            path.write_text('SYNTHETIC ' + name)
            return pin(path)
        generator, dataset, tokenizer, template = [asset(name) for name in ('generator', 'dataset', 'tokenizer', 'template')]
        tokenizer['revision'] = 'b' * 40
        receipt = self.root / 'receipt.json'
        receipt.write_text(json.dumps({'version': '1.8.1', 'builds': {'measured': {
            'binaries': {'metrum-ai-bench-cli-prompts': generator}}}}))
        lane = self.root / 'lane.json'
        lane.write_text(json.dumps({'lanes': {'L1': {'revision': 'b' * 40, 'template': template, 'prompt_sets': {'default': dataset}}}}))
        self.manifest = {'candidate_sha': 'a' * 40, 'lane': 'L1', 'lane_manifest': pin(lane),
            'build_receipt': pin(receipt), 'tokenizer': tokenizer,
            'cases': {'BC-LIVE-001': {'out': str(self.root / 'attempt')}},
            'inputs': [generator, dataset, tokenizer, template, pin(lane), pin(receipt)]}
        for target in ('check_manifest', 'load_receipt'):
            guard = patch.object(derived.build_roles, target)
            guard.start()
            self.addCleanup(guard.stop)
        self.manifest['derived_inputs'] = derived.declaration(self.manifest)
        self.input = self.root / 'attempt/evidence/l1/prompts.jsonl'
        self.input.parent.mkdir(parents=True)
        self.input.write_text('SYNTHETIC MATCHING PROMPTS\n')
        self.out = self.input.parent / 'input-binding'
        self.command = self.input.parent / 'prompt-command.json'
        self.args = ['--quiet', '--count', '16', '--seed', '7', '--output', str(self.input), '--report',
                     str(self.input.parent / 'prompt-mix.json'), '--local-jsonl', dataset['path'], '--profile', 'chat-short']
        self.command.write_text(json.dumps({'binary': generator['path'], 'argv': self.args}))

    def generate(self, argv, directory):
        (directory / 'prompts.jsonl').write_text('SYNTHETIC MATCHING PROMPTS\n')
        (directory / 'selection-report.json').write_text('{"SYNTHETIC":true}')
        (directory / 'generator.log').write_text('SYNTHETIC; NO GENERATOR EXECUTED\n')

    def test_internal_selection_binding_matches_original_before_workload(self):
        phases = self.root / 'phases'
        phases.mkdir()
        self.assertEqual(generated.preflight_label(self.manifest), 'STRUCTURAL-PREFLIGHT-OK')
        with patch.dict(derived.os.environ, {'CERT_PHASE_ROOT': str(phases)}, clear=True), \
             patch.object(generated, 'run_generator', side_effect=self.generate) as run:
            receipt = derived.boundary(self.manifest, self.command, self.input, self.out)
        self.assertEqual(json.loads((phases / 'phases.jsonl').read_text())['phase'], 'inputs-ready')
        self.assertEqual(run.call_count, 2)
        self.assertEqual(receipt['input'], pin(self.input))
        self.assertEqual(json.loads((self.out / 'original-binding.json').read_text()), receipt)
        ledger = derived.verify(self.manifest, self.root / 'attempt')
        self.assertEqual(ledger['input_sha256'], pin(self.input)['sha256'])
        self.input.write_text('TAMPERED AFTER BINDING')
        with self.assertRaises(ValueError):
            derived.verify(self.manifest, self.root / 'attempt')

    def test_report_requires_internal_boundary_and_records_actual_derived_hashes(self):
        import execution_evidence
        manifest_path = self.root / 'case.json'
        manifest_path.write_text(json.dumps(self.manifest))
        record = {'ID': 'BC-LIVE-001', 'candidate_sha': 'a' * 40, 'result': 'PASS', 'evidence_paths': []}
        env = {'CERT_INPUT_MANIFEST': str(manifest_path), 'CERT_CASE_ID': 'BC-LIVE-001'}
        with patch.dict(execution_evidence.os.environ, env, clear=True), self.assertRaises(ValueError):
            execution_evidence.enrich(self.root / 'attempt', record)
        with patch.object(generated, 'run_generator', side_effect=self.generate):
            derived.boundary(self.manifest, self.command, self.input, self.out)
        with patch.dict(execution_evidence.os.environ, env, clear=True):
            enriched = execution_evidence.enrich(self.root / 'attempt', record)
        self.assertEqual(enriched['versions']['derived_inputs']['l1']['input_sha256'], pin(self.input)['sha256'])

    def test_seed_or_profile_drift_stops_before_independent_generation(self):
        values = json.loads(self.command.read_text())
        values['argv'][values['argv'].index('--seed') + 1] = '42'
        self.command.write_text(json.dumps(values))
        with patch.object(generated, 'run_generator') as run, self.assertRaisesRegex(ValueError, 'config/seed/input'):
            derived.boundary(self.manifest, self.command, self.input, self.out)
        run.assert_not_called()
        bad = copy.deepcopy(self.manifest)
        bad['derived_inputs']['contract']['seed'] = 42
        with self.assertRaisesRegex(ValueError, 'frozen generator'):
            derived.validate(bad)

    def test_original_selection_mismatch_keeps_generations_but_blocks_workload(self):
        self.input.write_text('DIFFERENT SYNTHETIC ORIGINAL\n')
        with patch.object(generated, 'run_generator', side_effect=self.generate), self.assertRaisesRegex(ValueError, 'original selection differs'):
            derived.boundary(self.manifest, self.command, self.input, self.out)
        self.assertTrue((self.out / 'actual/prompts.jsonl').exists())
        self.assertFalse((self.out / 'original-binding.json').exists())

    def test_rust_hook_precedes_measured_command_and_keeps_original_generator(self):
        text = (Path(__file__).resolve().parents[3] / 'xtask/src/local_smoke.rs').read_text()
        original = text.index('let mut prompt_command = Command::new(&prompt_bin)')
        hook = text.index('bind certification-derived prompts')
        report = text.index('let report: Value = serde_json::from_slice(&fs::read(&report_path)?)?;')
        self.assertLess(original, hook)
        isolated = text.index('prompt_command.env_clear().envs(')
        output = text.index('let prompt_output = prompt_command.output()')
        self.assertLess(isolated, output)
        self.assertIn('certification prompt selection requires explicit local JSONL', text[original:output])
        self.assertLess(hook, report)
        self.assertIn('"--prompts".into(),', text[report:])
        self.assertIn('env::var("PROMPT_SEED").unwrap_or_else(|_| "7".into())', text)


if __name__ == '__main__':
    unittest.main()
