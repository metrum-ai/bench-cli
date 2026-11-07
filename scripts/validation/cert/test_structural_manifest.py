# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic structural-vs-concrete lane checks; never GPU/preflight evidence."""
import json
import tempfile
import unittest
from pathlib import Path
import manifest_tool


class StructuralManifestTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.file = self.root / 'lane.json'
        binary = manifest_tool.sha256_file('/bin/true')
        self.config = {'image': 'synthetic/image@sha256:' + '0' * 64, 'model': 'Qwen/Qwen3-8B',
                       'revision': 'b' * 40, 'port': 19001, 'proxy_port': 19002,
                       'server_flags': ['--reasoning-parser', 'qwen3', '--no-enable-prefix-caching'],
                       'owner': 'fixture', 'evidence_link': 'fixture',
                       'prompt_sets': {'default': {'generated_input': 'prompts'}}}
        self.data = {'candidate_sha': 'a' * 40, 'candidate_binary_sha256': binary,
                     'baseline_binary_sha256': binary, 'lanes': {'G4': self.config}}

    def load(self, ids=None):
        self.file.write_text(json.dumps(self.data))
        return manifest_tool.load_manifest(self.file, 'G4', '/bin/true', '/bin/true', '0', structural_recipe_ids=ids)

    def test_symbolic_inputs_are_explicit_structural_only_and_never_runtime_ready(self):
        _, config = self.load({'prompts'})
        self.assertEqual(config['prompt_sets'], {'default': {'generated_input': 'prompts'}})
        with self.assertRaisesRegex(ValueError, 'runtime needs concrete hashes'):
            self.load()
        output = self.root / 'actual.jsonl'
        output.write_text('SYNTHETIC ACTUAL BYTES\n')
        self.config['prompt_sets'] = {'default': {'path': str(output), 'sha256': manifest_tool.sha256_file(output)}}
        self.load()  # Original concrete path still validates real bytes.
        output.write_text('changed after runtime binding')
        with self.assertRaisesRegex(ValueError, 'hash mismatch'):
            self.load()

    def test_unknown_unused_duplicate_and_unvalidated_references_fail(self):
        for ids in (set(), {'other'}, {'prompts', 'unused'}, ['prompts']):
            with self.subTest(ids=ids), self.assertRaises(ValueError):
                self.load(ids)
        self.config['prompt_sets']['second'] = {'generated_input': 'prompts'}
        with self.assertRaisesRegex(ValueError, 'unique validated'):
            self.load({'prompts'})

    def test_structural_mode_keeps_model_and_server_criteria(self):
        self.config['server_flags'].remove('--no-enable-prefix-caching')
        with self.assertRaisesRegex(ValueError, 'disabled prefix caching'):
            self.load({'prompts'})
        self.config['server_flags'].append('--no-enable-prefix-caching')
        self.config['model'] = 'synthetic/other-model'
        with self.assertRaisesRegex(ValueError, 'pinned Qwen3-8B'):
            self.load({'prompts'})


if __name__ == '__main__':
    unittest.main()
