# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline SUT input preparation tests; not physical GPU evidence."""
import copy
import json
import tempfile
import unittest
from pathlib import Path

import manifest_tool
import prepare_product_sut


class ProductSutTests(unittest.TestCase):
    def setUp(self):
        self.actual = {'gpus': [{'name': 'Synthetic GPU', 'memory_gb': 48.9, 'driver': '550.1'}],
                       'memory_gb': 64.5, 'host_os': 'Synthetic Linux'}
        self.config = {'model': 'Qwen/Qwen3-8B', 'revision': 'a' * 40,
                       'image': 'vllm/vllm-openai@sha256:' + 'b' * 64,
                       'server_flags': ['--max-model-len', '16384']}

    def test_schema_and_truthful_conservative_inventory(self):
        sut = prepare_product_sut.declaration(self.actual, self.config)
        self.assertEqual(sut['gpu'], {'model': 'Synthetic GPU', 'count': 1, 'memory_gb': 48})
        self.assertEqual(sut['memory_gb'], 64)
        self.assertEqual(sut['model'], {'id': self.config['model'], 'revision': self.config['revision']})
        self.assertEqual(sut['field_provenance']['gpu'], 'observed')
        self.assertEqual(sut['field_provenance']['model'], 'declared')
        self.assertEqual(json.loads(sut['runtime']['config']), self.config['server_flags'])
        self.assertNotIn('name', sut['gpu'])

    def test_rejects_missing_or_invalid_observations(self):
        for field, value in [('gpus', []), ('gpus', [self.actual['gpus'][0]] * 2),
                             ('memory_gb', 0), ('memory_gb', float('nan')), ('memory_gb', True),
                             ('host_os', '')]:
            actual = copy.deepcopy(self.actual)
            actual[field] = value
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                prepare_product_sut.declaration(actual, self.config)
        for field, value in [('name', ''), ('driver', None), ('memory_gb', -1)]:
            actual = copy.deepcopy(self.actual)
            actual['gpus'][0][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                prepare_product_sut.declaration(actual, self.config)

    def test_rejects_unpinned_and_overriding_server_identity(self):
        for field, value in [('model', 'invalid model!'), ('revision', 'main'), ('image', 'vllm:latest'),
                             ('server_flags', ['--model', 'other'])]:
            config = {**self.config, field: value}
            with self.subTest(field=field), self.assertRaises(ValueError):
                prepare_product_sut.declaration(self.actual, config)

    def test_pins_and_exclusive_output(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            actual, lane, out = root / 'actual.json', root / 'lane.json', root / 'sut.json'
            actual.write_text(json.dumps(self.actual))
            lane.write_text(json.dumps({'lanes': {'L1': self.config}}))
            args = [actual, manifest_tool.sha256_file(actual), lane, manifest_tool.sha256_file(lane), 'L1', out]
            pin = prepare_product_sut.prepare(*args)
            self.assertEqual(pin['sha256'], manifest_tool.sha256_file(out))
            with self.assertRaises(FileExistsError):
                prepare_product_sut.prepare(*args)
            args[1] = '0' * 64
            with self.assertRaises(ValueError):
                prepare_product_sut.prepare(*args)


if __name__ == '__main__':
    unittest.main()
