# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic archives/API responses test the importer; no real workflow evidence."""
import copy
import io
import json
import shutil
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
import pretag_artifact as pretag
from build_role_test_fixture import workflow_evidence
from execution_evidence import digest


class PretagArtifactTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.sha = 'a' * 40
        self.origin = workflow_evidence(self.root / 'fixture', self.sha, '1.8.1',
            {'metrum-ai-bench-cli-llm': b'#!/bin/sh\n# synthetic unit fixture\nexit 0\n'})
        self.run = json.loads(Path(self.origin['run']['path']).read_text())
        self.artifact = json.loads(Path(self.origin['artifact_metadata']['path']).read_text())

    def test_verified_archive_and_mocked_gh_download(self):
        expected = pretag.verify(self.origin, self.sha)
        def transfer(argv, **kwargs):
            self.assertEqual(argv[:3], ['gh', 'run', 'download'])
            self.assertIn(pretag.ARTIFACT, argv)
            destination = Path(argv[argv.index('--dir') + 1])
            destination.mkdir()
            for field in ('archive', 'checksum'):
                source = Path(self.origin[field]['path'])
                shutil.copyfile(source, destination / source.name)
            kwargs['stdout'].write(b'SYNTHETIC TRANSPORT; NO REAL WORKFLOW\n')
        listing = {'total_count': 1, 'artifacts': [self.artifact]}
        with patch.object(pretag.subprocess, 'check_output', side_effect=[json.dumps(self.run), json.dumps(listing)]), \
             patch.object(pretag.subprocess, 'run', side_effect=transfer):
            origin, paths = pretag.download(self.sha, '1.8.1', 19, self.root / 'downloaded')
        self.assertEqual(pretag.verify(origin, self.sha), expected)
        self.assertEqual({name: digest(path) for name, path in paths.items()}, expected)

    def test_wrong_run_source_workflow_event_status_or_repository(self):
        for key, value in [('head_sha', 'b' * 40), ('id', 20), ('path', 'release.yml'),
                           ('event', 'pull_request'), ('status', 'in_progress'),
                           ('conclusion', 'failure'), ('head_repository', {'full_name': 'other/repo'})]:
            with self.subTest(key=key):
                run = copy.deepcopy(self.run)
                run[key] = value
                with self.assertRaisesRegex(ValueError, 'pretag workflow'):
                    pretag.validate_metadata(run, self.artifact, self.sha, 19)
        for key, value in [('name', 'other'), ('expired', True), ('workflow_run', {'id': 20, 'head_sha': self.sha})]:
            bad = copy.deepcopy(self.artifact)
            bad[key] = value
            with self.assertRaisesRegex(ValueError, 'artifact'):
                pretag.validate_metadata(self.run, bad, self.sha, 19)

    def test_local_origin_and_mutated_evidence_rejected(self):
        bad = copy.deepcopy(self.origin)
        bad['kind'] = 'local-cargo-measured'
        with self.assertRaisesRegex(ValueError, 'local measured'):
            pretag.verify(bad, self.sha)
        for field in ('run', 'artifact_metadata', 'artifact_listing', 'archive', 'checksum', 'download_log'):
            path = Path(self.origin[field]['path'])
            original = path.read_bytes()
            path.write_bytes(original + b'changed')
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'hash mismatch'):
                pretag.verify(self.origin, self.sha)
            path.write_bytes(original)

    def test_recorded_checksum_mismatch_rejected_even_if_pin_updated(self):
        path = Path(self.origin['checksum']['path'])
        path.write_text('0' * 64 + '  ' + Path(self.origin['archive']['path']).name + '\n')
        self.origin['checksum'] = pretag.pin(path)
        with self.assertRaisesRegex(ValueError, 'recorded checksum'):
            pretag.verify(self.origin, self.sha)

    def test_unsafe_link_traversal_or_duplicate_archive_member_rejected(self):
        archive = Path(self.origin['archive']['path'])
        root = 'metrum-ai-bench-cli-v1.8.1-' + pretag.TARGET
        for mode in ('link', 'traversal', 'duplicate'):
            with self.subTest(mode=mode):
                with tarfile.open(archive, 'w:gz') as tar:
                    member = tarfile.TarInfo(root + '/bin/metrum-ai-bench-cli-llm' if mode != 'traversal' else '../escape')
                    member.mode = 0o755
                    if mode == 'link':
                        member.type = tarfile.SYMTYPE
                        member.linkname = '/bin/sh'
                        tar.addfile(member)
                    else:
                        member.size = 1
                        tar.addfile(member, io.BytesIO(b'x'))
                        if mode == 'duplicate':
                            tar.addfile(member, io.BytesIO(b'x'))
                checksum = Path(self.origin['checksum']['path'])
                checksum.write_text(digest(archive) + '  ' + archive.name + '\n')
                self.origin.update(archive=pretag.pin(archive), checksum=pretag.pin(checksum))
                with self.assertRaises(ValueError):
                    pretag.verify(self.origin, self.sha)

    def test_invalid_inputs_do_not_access_network_or_create_outputs(self):
        for sha, version, run_id in [('x', '1.8.1', 19), (self.sha, '../escape', 19),
                                      (self.sha, '01.8.1', 19), (self.sha, '1.8.1', True)]:
            out = self.root / 'invalid'
            with patch.object(pretag.subprocess, 'check_output') as api, self.assertRaises(ValueError):
                pretag.download(sha, version, run_id, out)
            api.assert_not_called()
            self.assertFalse(out.exists())

    def test_duplicate_or_incomplete_artifact_listing_never_downloads(self):
        for index, listing in enumerate([{'total_count': 2, 'artifacts': [self.artifact]},
                                        {'total_count': 2, 'artifacts': [self.artifact, self.artifact]}]):
            with patch.object(pretag.subprocess, 'check_output', side_effect=[json.dumps(self.run), json.dumps(listing)]), \
                 patch.object(pretag.subprocess, 'run') as download, self.assertRaises(ValueError):
                pretag.download(self.sha, '1.8.1', 19, self.root / f'bad-list-{index}')
            download.assert_not_called()

    def test_failed_workflow_preserves_metadata_and_never_downloads(self):
        failed = dict(self.run, conclusion='failure')
        listing = {'total_count': 1, 'artifacts': [self.artifact]}
        out = self.root / 'failed'
        with patch.object(pretag.subprocess, 'check_output', side_effect=[json.dumps(failed), json.dumps(listing)]), \
             patch.object(pretag.subprocess, 'run') as download, self.assertRaises(ValueError):
            pretag.download(self.sha, '1.8.1', 19, out)
        download.assert_not_called()
        self.assertEqual(json.loads((out / 'run.json').read_text())['conclusion'], 'failure')
        self.assertTrue((out / 'artifacts.json').exists())


if __name__ == '__main__':
    unittest.main()
