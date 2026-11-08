# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic receipt fixtures only; NEVER actual build/certification evidence."""
import copy
import json
import io
import tarfile
import tempfile
import unittest
from pathlib import Path
import build_roles
import pretag_artifact
from build_role_test_fixture import bind, workflow_evidence
from execution_evidence import digest


class VariantBuildTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.sha = 'a' * 40
        binary = self.root / 'shipped'
        binary.write_bytes(b'#!/bin/sh\n# synthetic shipped\nexit 0\n')
        binary.chmod(0o755)
        manifest = {'candidate_sha': self.sha, 'candidate': {'path': str(binary), 'sha256': digest(binary)}}
        bind(manifest, self.root)
        self.receipt_path = Path(manifest['build_receipt']['path'])
        self.receipt = json.loads(self.receipt_path.read_text())
        variant = self.root / 'variant'
        variant.write_bytes(b'#!/bin/sh\n# synthetic tokenizer variant\nexit 0\n')
        variant.chmod(0o755)
        origin = workflow_evidence(self.root / 'variant-workflow', self.sha, '1.8.1',
                                  {'metrum-ai-bench-cli-llm': variant.read_bytes()}, variant=True)
        self.item = {'path': str(variant), 'sha256': digest(variant), 'candidate_sha': self.sha,
                     'role': 'feature-variant-tokenizer', 'profile': 'release',
                     'features': ['otlp', 'tokenizer'], 'origin': origin}
        self.receipt['builds']['feature-variant-tokenizer'] = {
            'origin': origin, 'binaries': {'metrum-ai-bench-cli-llm': self.item}}

    def save(self, receipt):
        self.receipt_path.write_text(json.dumps(receipt))

    def test_verified_variant_receipt_is_distinct_from_shipped_and_validation(self):
        self.save(self.receipt)
        self.assertEqual(build_roles.load_variant(self.receipt_path, self.sha), self.item)
        self.assertEqual(self.item['origin']['artifact_name'], pretag_artifact.TOKENIZER_ARTIFACT)

    def test_wrong_source_features_role_hash_and_profile_fail(self):
        for field, value in [('candidate_sha', 'b' * 40), ('features', ['otlp']),
                             ('role', 'validation-only'), ('sha256', '0' * 64), ('profile', 'debug')]:
            with self.subTest(field=field):
                bad = copy.deepcopy(self.receipt)
                bad['builds']['feature-variant-tokenizer']['binaries']['metrum-ai-bench-cli-llm'][field] = value
                self.save(bad)
                with self.assertRaises(ValueError):
                    build_roles.load_variant(self.receipt_path, self.sha)

    def test_missing_variant_or_borrowed_validation_binary_cannot_pass(self):
        bad = copy.deepcopy(self.receipt)
        del bad['builds']['feature-variant-tokenizer']
        self.save(bad)
        with self.assertRaises(ValueError):
            build_roles.load_variant(self.receipt_path, self.sha)
        bad = copy.deepcopy(self.receipt)
        item = bad['builds']['feature-variant-tokenizer']['binaries']['metrum-ai-bench-cli-llm']
        helper = bad['builds']['validation-only']['binaries']['metrum-ai-bench-cli-llm']
        item.update(path=helper['path'], sha256=helper['sha256'])
        self.save(bad)
        with self.assertRaisesRegex(ValueError, 'workflow'):
            build_roles.load_variant(self.receipt_path, self.sha)

    def test_validation_identity_rejected_even_with_separately_valid_archive_claim(self):
        helper = self.receipt['builds']['validation-only']['binaries']['metrum-ai-bench-cli-llm']
        copied = self.root / 'copied-helper'
        copied.write_bytes(Path(helper['path']).read_bytes())
        copied.chmod(0o755)
        for index, path in enumerate([Path(helper['path']), copied]):
            with self.subTest(path=path.name):
                origin = workflow_evidence(self.root / ('borrowed-' + str(index)), self.sha, '1.8.1',
                    {'metrum-ai-bench-cli-llm': path.read_bytes()}, variant=True)
                # Synthetic adversarial claim: all archive/source hashes agree,
                # but validation identity/bytes must still never become measured.
                pretag_artifact.verify(origin, self.sha, variant=True)
                bad = copy.deepcopy(self.receipt)
                entry = bad['builds']['feature-variant-tokenizer']
                entry['origin'] = origin
                entry['binaries']['metrum-ai-bench-cli-llm'].update(
                    path=str(path), sha256=digest(path), origin=origin)
                self.save(bad)
                with self.assertRaisesRegex(ValueError, 'validation build'):
                    build_roles.load_variant(self.receipt_path, self.sha)

    def test_missing_or_false_archive_build_declaration_rejected_with_updated_hashes(self):
        origin = copy.deepcopy(self.item['origin'])
        archive = Path(origin['archive']['path'])
        root = 'metrum-ai-bench-cli-v1.8.1-' + pretag_artifact.TARGET + '-tokenizer'
        correct = pretag_artifact.variant_profile(self.sha, '1.8.1')
        changes = [None, {'features': ['otlp']}, {'candidate_sha': 'b' * 40},
                   {'profile': 'debug'}, {'version': '1.8.0'}, {'target': 'other'},
                   {'label': 'shipped build'}, {'argv': ['cargo', 'build']}, {'unexpected': True}]
        for change in changes:
            with self.subTest(change=change):
                with tarfile.open(archive, 'w:gz') as tar:
                    members = {root + '/bin/metrum-ai-bench-cli-llm': Path(self.item['path']).read_bytes()}
                    if change is not None:
                        members[root + '/build-profile.json'] = json.dumps({**correct, **change}).encode()
                    for name, data in members.items():
                        info = tarfile.TarInfo(name)
                        info.mode = 0o755
                        info.size = len(data)
                        tar.addfile(info, io.BytesIO(data))
                checksum = Path(origin['checksum']['path'])
                checksum.write_text(digest(archive) + '  ' + archive.name + '\n')
                origin.update(archive=pretag_artifact.pin(archive), checksum=pretag_artifact.pin(checksum))
                with self.assertRaisesRegex(ValueError, 'variant build profile'):
                    pretag_artifact.verify(origin, self.sha, variant=True)

    def test_nonpublishing_workflow_keeps_shipping_profile_and_separate_variant(self):
        workflow = (Path(__file__).resolve().parents[3] / '.github/workflows/pretag-certification.yml').read_text()
        self.assertIn('cargo zigbuild --locked --release --features otlp,tokenizer --target "${{ matrix.zig_target }}" --bins', workflow)
        self.assertIn('cargo zigbuild --locked --release --features otlp,tokenizer --target x86_64-unknown-linux-gnu.2.17 --bin metrum-ai-bench-cli-llm', workflow)
        self.assertIn('name: pretag-tokenizer-x86_64-unknown-linux-gnu', workflow)
        self.assertIn("'label': 'feature-variant build (tokenizer)'", workflow)
        self.assertNotIn('gh release create', workflow)


if __name__ == '__main__':
    unittest.main()
