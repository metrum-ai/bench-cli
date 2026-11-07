# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic executable fixtures test role guards, not build or GPU evidence."""
import copy
import tempfile
import unittest
from pathlib import Path
import build_roles
from execution_evidence import digest


class BuildRoleTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.sha = 'a' * 40
        self.manifest = {'candidate_sha': self.sha}
        for key, role, features in [('candidate', 'measured', ['otlp']),
                                   ('tokenizer_validation', 'validation-only', ['otlp', 'tokenizer'])]:
            path = self.root / key
            path.write_text('#!/bin/sh\n# Copyright (c) 2026 Metrum AI, Inc.\n# ' + role + '\nexit 0\n')
            path.chmod(0o755)
            self.manifest[key] = {'path': str(path), 'sha256': digest(path), 'candidate_sha': self.sha,
                                  'profile': 'release', 'features': features, 'role': role,
                                  'origin': {'kind': 'pretag-workflow' if role == 'measured' else 'local-cargo-validation'}}

    def test_same_source_distinct_roles(self):
        paths = build_roles.check_roles(self.manifest)
        self.assertEqual(paths, tuple(Path(self.manifest[k]['path']) for k in ('candidate', 'tokenizer_validation')))
        build_roles.reject_validation_use(['--candidate', str(paths[0])], self.manifest['tokenizer_validation'])

    def test_wrong_features_source_profile_role_and_hash_fail(self):
        for key in ('candidate', 'tokenizer_validation'):
            for field, value in [('features', []), ('candidate_sha', 'b' * 40), ('profile', 'debug'),
                                 ('role', 'other'), ('sha256', '0' * 64), ('origin', {'kind': 'local-measured'})]:
                with self.subTest(key=key, field=field):
                    bad = copy.deepcopy(self.manifest)
                    bad[key][field] = value
                    with self.assertRaises(ValueError):
                        build_roles.check_roles(bad)
        bad = copy.deepcopy(self.manifest)
        del bad['tokenizer_validation']
        with self.assertRaisesRegex(ValueError, 'metadata'):
            build_roles.check_roles(bad)

    def test_alias_or_same_bytes_cannot_serve_both_roles(self):
        bad = copy.deepcopy(self.manifest)
        bad['tokenizer_validation'].update({k: bad['candidate'][k] for k in ('path', 'sha256')})
        with self.assertRaisesRegex(ValueError, 'distinct'):
            build_roles.check_roles(bad)

    def test_secondary_and_future_cli_slots_require_measured_registry(self):
        measured = [self.manifest['candidate']]
        validation = self.manifest['tokenizer_validation']
        other = self.root / 'unregistered'
        other.write_text('#!/bin/sh\n# Copyright (c) 2026 Metrum AI, Inc.\n# other\nexit 0\n')
        other.chmod(0o755)
        unregistered = {'path': str(other), 'sha256': digest(other)}
        for field in sorted(build_roles.MEASURED_CLI_FIELDS | {'future_cli_binary'}):
            with self.subTest(field=field):
                build_roles.check_measured_config({'lane': {field: measured[0]}}, measured, validation)
                with self.assertRaisesRegex(ValueError, 'registry'):
                    build_roles.check_measured_config({'lane': {field: unregistered}}, measured, validation)
                with self.assertRaisesRegex(ValueError, 'validation-only'):
                    build_roles.check_measured_config({'lane': {field: validation}}, measured, validation)
        # External telemetry tooling is pinned independently, not a bench CLI.
        build_roles.check_measured_config({'all_smi_binary': unregistered}, measured, validation)

    def test_receipt_and_lane_binding_reject_mutation_before_direct_launch(self):
        from build_role_test_fixture import bind
        source = copy.deepcopy(self.manifest)
        source['cases'] = {}
        manifest = bind(source, self.root)
        build_roles.check_manifest(manifest)
        lane = Path(manifest['lane_manifest']['path'])
        lane.write_text('{}')
        with self.assertRaisesRegex(ValueError, 'lane manifest hash'):
            build_roles.check_manifest(manifest)
        manifest['lane_manifest']['sha256'] = digest(lane)
        receipt = Path(manifest['build_receipt']['path'])
        receipt.write_text('{}')
        with self.assertRaisesRegex(ValueError, 'receipt hash'):
            build_roles.check_manifest(manifest)

    def test_arbitrary_binary_cannot_borrow_valid_workflow_origin(self):
        import json
        from build_role_test_fixture import bind
        source = copy.deepcopy(self.manifest)
        source['cases'] = {}
        manifest = bind(source, self.root)
        primary = Path(manifest['candidate']['path'])
        primary.write_text('#!/bin/sh\n# different locally compiled variant\nexit 0\n')
        manifest['candidate']['sha256'] = digest(primary)
        receipt = Path(manifest['build_receipt']['path'])
        data = json.loads(receipt.read_text())
        data['builds']['measured']['binaries']['metrum-ai-bench-cli-llm']['sha256'] = digest(primary)
        receipt.write_text(json.dumps(data))
        manifest['build_receipt']['sha256'] = digest(receipt)
        with self.assertRaisesRegex(ValueError, 'verified workflow archive'):
            build_roles.check_manifest(manifest)

    def test_validation_binary_and_copy_rejected_in_measured_commands_or_pins(self):
        item = self.manifest['tokenizer_validation']
        duplicate = self.root / 'renamed-helper'
        duplicate.write_bytes(Path(item['path']).read_bytes())
        duplicate.chmod(0o755)
        alias = self.root / 'alias'
        alias.symlink_to(item['path'])
        for value in ([item['path'], '--out', '/unused'], [str(duplicate)], [str(alias)],
                      {'nested': [{'path': str(duplicate), 'sha256': item['sha256']}]}):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, 'validation-only'):
                build_roles.reject_validation_use(value, item)


if __name__ == '__main__':
    unittest.main()
