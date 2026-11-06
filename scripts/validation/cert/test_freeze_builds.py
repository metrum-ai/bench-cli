# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Mock compiler outputs exercise receipt generation, not release evidence."""
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch
import freeze_builds
import build_roles


class FreezeBuildTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / 'repo'
        self.repo.mkdir()
        self.args = SimpleNamespace(repo=self.repo, out=self.root / 'builds', candidate_sha='a' * 40,
                                    rust_target='x86_64-unknown-linux-gnu',
                                    zig_target='x86_64-unknown-linux-gnu.2.17')
        self.names = ['metrum-ai-bench-cli-llm', 'metrum-ai-bench-cli-strategic']
        self.metadata = json.dumps({'packages': [{'name': 'metrum-ai-bench-cli',
            'manifest_path': str(self.repo / 'Cargo.toml'),
            'targets': [{'kind': ['bin'], 'name': name} for name in self.names]}]}).encode()

    def compiler(self, argv, **kwargs):
        target = Path(argv[argv.index('--target-dir') + 1])
        features = argv[argv.index('--features') + 1]
        names = self.names if '--bins' in argv else ['metrum-ai-bench-cli-llm']
        for name in names:
            path = target / self.args.rust_target / 'release' / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('#!/bin/sh\n# Copyright (c) 2026 Metrum AI, Inc.\n# synthetic ' + features + '\nexit 0\n')
            path.chmod(0o755)
        kwargs['stdout'].write(b'synthetic compiler fixture; not build evidence\n')

    def test_receipt_binds_roles_features_source_logs_and_separate_outputs(self):
        with patch.object(freeze_builds, 'clean_source') as clean, \
             patch.object(freeze_builds.subprocess, 'check_output', return_value=self.metadata), \
             patch.object(freeze_builds.subprocess, 'run', side_effect=self.compiler) as compile:
            result = freeze_builds.build(self.args)
        self.assertEqual(clean.call_count, 3)
        self.assertEqual(compile.call_count, 2)
        primary = result['builds']['measured']['binaries']
        validation = result['builds']['validation-only']['binaries']['metrum-ai-bench-cli-llm']
        self.assertEqual(set(primary), set(self.names))
        for item in primary.values():
            self.assertEqual(item['features'], ['otlp'])
            self.assertEqual(item['role'], 'measured')
            self.assertEqual(item['candidate_sha'], self.args.candidate_sha)
        self.assertEqual(validation['features'], ['otlp', 'tokenizer'])
        self.assertNotEqual(validation['path'], primary['metrum-ai-bench-cli-llm']['path'])
        self.assertNotEqual(validation['sha256'], primary['metrum-ai-bench-cli-llm']['sha256'])
        self.assertEqual(result, json.loads((self.args.out / 'builds.json').read_text()))
        registry, helper = build_roles.load_receipt(self.args.out / 'builds.json', self.args.candidate_sha)
        self.assertEqual(registry, list(primary.values()))
        self.assertEqual(helper, validation)
        with self.assertRaises(FileExistsError), patch.object(freeze_builds, 'clean_source'):
            freeze_builds.build(self.args)

    def test_failed_second_build_retains_logs_without_completed_receipt(self):
        def compile(argv, **kwargs):
            if 'otlp,tokenizer' in argv:
                kwargs['stdout'].write(b'synthetic failed compile\n')
                raise subprocess.CalledProcessError(1, argv)
            return self.compiler(argv, **kwargs)
        with patch.object(freeze_builds, 'clean_source'), \
             patch.object(freeze_builds.subprocess, 'check_output', return_value=self.metadata), \
             patch.object(freeze_builds.subprocess, 'run', side_effect=compile), \
             self.assertRaises(subprocess.CalledProcessError):
            freeze_builds.build(self.args)
        self.assertFalse((self.args.out / 'builds.json').exists())
        self.assertTrue((self.args.out / 'validation-only.log').is_file())

    def test_changed_source_or_default_features_rejected(self):
        with patch.object(freeze_builds, 'git', return_value='b' * 40), self.assertRaisesRegex(ValueError, 'frozen'):
            freeze_builds.clean_source(self.repo, self.args.candidate_sha)
        (self.repo / 'Cargo.toml').write_text('[features]\ndefault=["tokenizer"]\n')
        with patch.object(freeze_builds, 'git', side_effect=[self.args.candidate_sha, '']), self.assertRaisesRegex(ValueError, 'default features'):
            freeze_builds.clean_source(self.repo, self.args.candidate_sha)


if __name__ == '__main__':
    unittest.main()
