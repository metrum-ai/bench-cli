# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Optional real-binary loopback parser tests; never certification evidence."""
import json
import os
import tempfile
import unittest
from pathlib import Path

import manifest_tool
import prepare_product_sut
import rehearse_product_llm


@unittest.skipUnless(os.environ.get('CERT_TEST_LLM'), 'set CERT_TEST_LLM to exercise the actual binary parser')
class ProductParserTests(unittest.TestCase):
    def test_good_sut_and_negative_controls(self):
        binary = Path(os.environ['CERT_TEST_LLM']).resolve(strict=True)
        actual = {'gpus': [{'name': 'Synthetic GPU', 'memory_gb': 48, 'driver': 'synthetic'}],
                  'memory_gb': 64, 'host_os': 'Synthetic Linux'}
        config = {'model': 'Qwen/Qwen3-8B', 'revision': 'a' * 40,
                  'image': 'vllm/vllm-openai@sha256:' + 'b' * 64, 'server_flags': []}
        sut = prepare_product_sut.declaration(actual, config)
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'sut.json'
            path.write_text(json.dumps(sut))
            binary_sha = manifest_tool.sha256_file(binary)
            rehearse_product_llm.probe(binary, binary_sha, path, manifest_tool.sha256_file(path))
            with self.assertRaisesRegex(ValueError, 'hash mismatch'):
                rehearse_product_llm.probe(binary, '0' * 64, path, manifest_tool.sha256_file(path))
            sut['gpu']['name'] = sut['gpu'].pop('model')  # Certification schema, not product schema.
            path.write_text(json.dumps(sut))
            with self.assertRaisesRegex(ValueError, 'probe failed'):
                rehearse_product_llm.probe(binary, binary_sha, path, manifest_tool.sha256_file(path))


if __name__ == '__main__':
    unittest.main()
