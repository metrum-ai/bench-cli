# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline OCI index/child/config identity and mismatch controls."""
import hashlib
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from types import SimpleNamespace

import r4_image_provenance as provenance


def digest(data):
    return 'sha256:' + hashlib.sha256(data).hexdigest()


class OCIProvenanceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.config_digest = 'sha256:' + 'a' * 64
        self.child_raw = json.dumps({'schemaVersion': 2, 'config': {'digest': self.config_digest}}).encode()
        self.child_digest = digest(self.child_raw)
        self.child = self.root / 'child.json'
        self.child.write_bytes(self.child_raw)
        self.entry = {'digest': self.child_digest, 'size': len(self.child_raw),
                      'platform': {'os': 'linux', 'architecture': 'amd64'}}
        self.image = {'Id': self.config_digest, 'Os': 'linux', 'Architecture': 'amd64'}
        self.container = {'Image': self.config_digest}

    def index(self, entries):
        path = self.root / 'index.json'
        raw = json.dumps({'schemaVersion': 2, 'manifests': entries}).encode()
        path.write_bytes(raw)
        return path, digest(raw)

    def check(self, entries, image=None, container=None):
        path, index_digest = self.index(entries)
        return provenance.verify(path, self.child, image or self.image, container,
                                 index_digest=index_digest, child_digest=self.child_digest)

    def test_match_and_container_binding(self):
        result = self.check([self.entry], container=self.container)
        self.assertTrue(result['container_verified'])
        self.assertEqual(result['platform_manifest'], self.child_digest)
        with self.assertRaisesRegex(ValueError, 'container'):
            self.check([self.entry], container={'Image': 'sha256:' + 'b' * 64})
        for field, value in [('Id', 'sha256:' + 'b' * 64), ('Os', 'windows'), ('Architecture', 'arm64')]:
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'local image'):
                self.check([self.entry], image={**self.image, field: value})

    def test_bad_membership_and_corrupted_descriptor(self):
        for entries in ([], [self.entry, self.entry], [{**self.entry, 'size': 1}],
                        [{**self.entry, 'digest': 'sha256:' + '0' * 64}]):
            with self.subTest(entries=entries), self.assertRaises(ValueError):
                self.check(entries)
        index, index_digest = self.index([self.entry])
        self.child.write_bytes(self.child_raw + b' ')
        with self.assertRaisesRegex(ValueError, 'digest mismatch'):
            provenance.verify(index, self.child, self.image, index_digest=index_digest, child_digest=self.child_digest)

    def test_runtime_requires_staged_files_and_records_actual_container(self):
        index, index_digest = self.index([self.entry])
        output = self.root / 'receipt.json'
        def docker(command, **kwargs):
            obj = self.image if command[1:3] == ['image', 'inspect'] else self.container
            return SimpleNamespace(stdout=json.dumps([obj]))
        with patch.dict('os.environ', {'CERT_R4_OCI_INDEX': str(index), 'CERT_R4_OCI_AMD64': str(self.child)}), \
             patch.object(provenance, 'INDEX', index_digest), patch.object(provenance, 'AMD64', self.child_digest), \
             patch.object(provenance, 'verify', wraps=provenance.verify) as verify, \
             patch.object(provenance.subprocess, 'run', side_effect=docker):
            # Function defaults were frozen at definition; route synthetic test digests explicitly.
            def synthetic(index_path, child_path, image, container):
                return verify(index_path, child_path, image, container,
                              index_digest=index_digest, child_digest=self.child_digest)
            with patch.object(provenance, 'verify', side_effect=synthetic):
                result = provenance.runtime(provenance.pins.IMAGE, 'fixture-container', output)
            self.assertTrue(result['container_verified'])
            self.assertEqual(json.loads(output.read_text())['platform_manifest'], self.child_digest)
        output.unlink()
        with patch.dict('os.environ', {}, clear=True), self.assertRaisesRegex(ValueError, 'staged'):
            provenance.runtime(provenance.pins.IMAGE, 'fixture-container', output)
        self.assertFalse(output.exists())

    def test_smoke_runtime_checks_container_gpu_before_receipt(self):
        import result_writer
        index, index_digest = self.index([self.entry])
        uuid = 'GPU-11111111-1111-1111-1111-111111111111'
        (self.root / result_writer.SMOKE_MARKER).write_text(json.dumps({
            'classification': result_writer.SMOKE_CLASS, 'expectations_sha256': 'a' * 64}))
        evidence = self.root / 'evidence'; evidence.mkdir()
        output = evidence / 'r4-image-runtime.json'
        (evidence / 'gpu.csv').write_text('NVIDIA H100,550.1\n')
        request = {'DeviceIDs': [uuid], 'Count': 0, 'Driver': '', 'Capabilities': [['gpu']]}
        container = {**self.container, 'HostConfig': {'DeviceRequests': [request]}}
        def docker(command, **kwargs):
            if command[1] == 'exec':
                return SimpleNamespace(stdout=f'NVIDIA H100, {uuid}, 550.1\n')
            return SimpleNamespace(stdout=json.dumps([self.image if command[1:3] == ['image', 'inspect'] else container]))
        verify = provenance.verify
        def synthetic(index_path, child_path, image, container):
            return verify(index_path, child_path, image, container,
                          index_digest=index_digest, child_digest=self.child_digest)
        env = {'CERT_R4_OCI_INDEX': str(index), 'CERT_R4_OCI_AMD64': str(self.child), 'CUDA_VISIBLE_DEVICES': uuid}
        with patch.dict('os.environ', env), patch.object(provenance, 'verify', side_effect=synthetic), \
             patch.object(provenance.subprocess, 'run', side_effect=docker):
            receipt = provenance.runtime(provenance.pins.IMAGE, 'fixture', output)
            self.assertEqual(receipt['gpu_uuid'], uuid)
            self.assertEqual(receipt['gpu_visible'], {'name': 'NVIDIA H100', 'uuid': uuid, 'driver': '550.1'})
            output.unlink()
            (evidence / 'gpu.csv').write_text('NVIDIA RTX,550.1\n')
            with self.assertRaisesRegex(ValueError, 'host GPU identity'):
                provenance.runtime(provenance.pins.IMAGE, 'fixture', output)
            self.assertFalse(output.exists())
            (evidence / 'gpu.csv').write_text('NVIDIA H100,550.1\n')
            request['DeviceIDs'] = ['0']
            with self.assertRaisesRegex(ValueError, 'container GPU request'):
                provenance.runtime(provenance.pins.IMAGE, 'fixture', output)
            self.assertFalse(output.exists())

    def test_successful_records_require_bound_runtime_receipts(self):
        import execution_evidence
        evidence = self.root / 'evidence'
        evidence.mkdir()
        record = {'ID': 'BC-GPU-015', 'result': 'PASS', 'evidence_paths': [],
                  'versions': {'server_image': provenance.pins.IMAGE}}
        with self.assertRaisesRegex(ValueError, 'missing'):
            execution_evidence.bind_r4_evidence(self.root, record)
        data = {'image_index': provenance.INDEX, 'platform_manifest': provenance.AMD64,
                'platform': 'linux/amd64', 'image_config': provenance.CONFIG, 'container_verified': False}
        (evidence / 'r4-image-preflight.json').write_text(json.dumps(data))
        data['container_verified'] = True
        for name in ('r4-image-runtime.json', 'g5-endpoints-image-runtime.json'):
            (evidence / name).write_text(json.dumps(data))
        result = execution_evidence.bind_r4_evidence(self.root, record)
        self.assertEqual(len(result['evidence_paths']), 3)
        self.assertEqual(result['versions']['platform_manifest'], provenance.AMD64)
        data['image_config'] = 'sha256:' + 'b' * 64
        (evidence / 'g5-endpoints-image-runtime.json').write_text(json.dumps(data))
        with self.assertRaisesRegex(ValueError, 'invalid'):
            execution_evidence.bind_r4_evidence(self.root, record)

    def test_duplicate_json_keys_fail_closed(self):
        raw = b'{"manifests":[],"manifests":[]}'
        path = self.root / 'index.json'
        path.write_bytes(raw)
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            provenance.verify(path, self.child, self.image, index_digest=digest(raw), child_digest=self.child_digest)


if __name__ == '__main__':
    unittest.main()
