# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic transport/compiler fixtures, NOT real workflow or build evidence."""
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch
import freeze_builds
import build_roles
from build_role_test_fixture import workflow_evidence


class FreezeBuildTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / 'repo'
        self.repo.mkdir()
        self.args = SimpleNamespace(repo=self.repo, out=self.root / 'builds', candidate_sha='a' * 40,
                                    version='1.8.1', workflow_run_id=19)
        self.names = ['metrum-ai-bench-cli-llm', 'metrum-ai-bench-cli-strategic']

    def download(self, sha, version, run_id, out):
        content = b'#!/bin/sh\n# Copyright (c) 2026 Metrum AI, Inc.\n# synthetic workflow measured\nexit 0\n'
        origin = workflow_evidence(out, sha, version, {name: content for name in self.names}, run_id)
        paths = {}
        for name in self.names:
            path = out / name
            path.write_bytes(content)
            path.chmod(0o755)
            paths[name] = path
        return origin, paths

    def compiler(self, argv, **kwargs):
        self.assertEqual(argv[:2], ['cargo', 'build'])
        self.assertIn('otlp,tokenizer', argv)
        target = Path(argv[argv.index('--target-dir') + 1])
        path = target / 'release' / 'metrum-ai-bench-cli-llm'
        path.parent.mkdir(parents=True)
        path.write_text('#!/bin/sh\n# Copyright (c) 2026 Metrum AI, Inc.\n# synthetic local validation\nexit 0\n')
        path.chmod(0o755)
        kwargs['stdout'].write(b'synthetic compiler fixture; not build evidence\n')

    def test_workflow_measured_and_local_validation_receipt(self):
        with patch.object(freeze_builds, 'clean_source') as clean, \
             patch.object(freeze_builds.pretag_artifact, 'download', side_effect=self.download), \
             patch.object(freeze_builds.subprocess, 'run', side_effect=self.compiler) as compile:
            result = freeze_builds.build(self.args)
        self.assertEqual(clean.call_count, 3)
        self.assertEqual(compile.call_count, 1)  # No local measured compilation.
        primary = result['builds']['measured']['binaries']
        validation = result['builds']['validation-only']['binaries']['metrum-ai-bench-cli-llm']
        self.assertEqual(set(primary), set(self.names))
        self.assertTrue(all(item['origin']['kind'] == 'pretag-workflow' for item in primary.values()))
        self.assertEqual(validation['origin']['kind'], 'local-cargo-validation')
        self.assertNotEqual(validation['sha256'], primary[self.names[0]]['sha256'])
        self.assertEqual(result, json.loads((self.args.out / 'builds.json').read_text()))
        registry, helper = build_roles.load_receipt(self.args.out / 'builds.json', self.args.candidate_sha)
        self.assertEqual(registry, list(primary.values()))
        self.assertEqual(helper, validation)
        with self.assertRaises(FileExistsError), patch.object(freeze_builds, 'clean_source'):
            freeze_builds.build(self.args)

    def test_failed_validation_retains_downloads_and_log_without_receipt(self):
        with patch.object(freeze_builds, 'clean_source'), \
             patch.object(freeze_builds.pretag_artifact, 'download', side_effect=self.download), \
             patch.object(freeze_builds.subprocess, 'run', side_effect=subprocess.CalledProcessError(1, ['cargo'])), \
             self.assertRaises(subprocess.CalledProcessError):
            freeze_builds.build(self.args)
        self.assertFalse((self.args.out / 'builds.json').exists())
        self.assertTrue((self.args.out / 'validation-only.log').is_file())
        self.assertTrue(list((self.args.out / 'measured').glob('*.tar.gz')))

    def test_changed_source_defaults_or_version_rejected(self):
        with patch.object(freeze_builds, 'git', return_value='b' * 40), self.assertRaisesRegex(ValueError, 'frozen'):
            freeze_builds.clean_source(self.repo, self.args.candidate_sha)
        (self.repo / 'Cargo.toml').write_text('[features]\ndefault=["tokenizer"]\n')
        with patch.object(freeze_builds, 'git', side_effect=[self.args.candidate_sha, '']), self.assertRaisesRegex(ValueError, 'default features'):
            freeze_builds.clean_source(self.repo, self.args.candidate_sha)
        (self.repo / 'Cargo.toml').write_text('[package]\nversion="1.8.0"\n')
        with patch.object(freeze_builds, 'git', side_effect=[self.args.candidate_sha, '']), self.assertRaisesRegex(ValueError, 'version'):
            freeze_builds.clean_source(self.repo, self.args.candidate_sha, '1.8.1')


if __name__ == '__main__':
    unittest.main()
