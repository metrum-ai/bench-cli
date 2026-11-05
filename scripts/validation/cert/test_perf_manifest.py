# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline 27B manifest validation tests; all paths and assets are fixtures."""
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import manifest_tool
import perf_baseline


class PerfManifestTests(unittest.TestCase):
    def test_requires_fixed_model_prompt_binary_and_server_flags(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); w1 = root / 'W1.jsonl'; w2 = root / 'W2.jsonl'; asset = root / 'asset'
            for path in (w1, w2, asset): path.write_text('fixture')
            h1 = '4adc601afa3e4ce355a204dd042f1f64c3a24bdd7ef57a8a9c9ccd21d0ce0dd7'
            h2 = 'eb1ffdab20236c7aaadf63de3c03350076a47cccf796f279aacd551a38d8df8b'
            binary = manifest_tool.sha256_file('/bin/true'); asset_hash = manifest_tool.sha256_file(asset)
            config = {'model': 'nvidia/Qwen3.8-27B-NVFP4', 'revision': '482ca0f3832238542f8f5295dde86b5f22711d80',
                      'image': 'vllm/vllm-openai@sha256:befccf0e7e4089aa23926bb6f887a85c0d46c2a99e6de5c5ffdf5cb7957c5b34',
                      'port': 8000, 'server_flags': ['--max-model-len', '16384', '--max-num-seqs', '128', '--gpu-memory-utilization', '0.90',
                      '--default-chat-template-kwargs', '{"enable_thinking": false}', '--compilation-config', '{"cudagraph_mode":"PIECEWISE"}',
                      '--no-enable-prefix-caching'], 'owner': 'fixture', 'evidence_link': 'fixture',
                      'baseline_asset': {'path': str(asset), 'sha256': asset_hash},
                      'prompt_sets': {'W1': {'path': str(w1), 'sha256': h1}, 'W2': {'path': str(w2), 'sha256': h2}}}
            manifest = {'candidate_sha': 'a' * 40, 'candidate_binary_sha256': binary, 'baseline_binary_sha256': binary, 'lanes': {'P2': config}}
            file = root / 'manifest.json'; file.write_text(json.dumps(manifest))
            real_digest = manifest_tool.sha256_file
            def digest(path):
                if Path(path) == w1: return h1
                if Path(path) == w2: return h2
                return real_digest(path)
            with patch.object(manifest_tool, 'sha256_file', side_effect=digest), patch.object(perf_baseline, 'verify', return_value={}):
                for h1 in ('4adc601afa3e4ce355a204dd042f1f64c3a24bdd7ef57a8a9c9ccd21d0ce0dd7',
                           '44340640d8bf5818cc9467f4e50b23abf3cf329005999b9c77b736d3fb3fa340'):
                    for h2 in ('e5480af17428c48e614a0840f30eda53acbea7f9efe869c5173cde5d5818edfd',
                               'eb1ffdab20236c7aaadf63de3c03350076a47cccf796f279aacd551a38d8df8b'):
                        with self.subTest(w1=h1, w2=h2):
                            config['prompt_sets']['W1']['sha256'] = h1
                            config['prompt_sets']['W2']['sha256'] = h2
                            file.write_text(json.dumps(manifest))
                            manifest_tool.load_manifest(file, 'P2', '/bin/true', '/bin/true', '0')
                h2 = 'b' * 64
                config['prompt_sets']['W2']['sha256'] = h2; file.write_text(json.dumps(manifest))
                with self.assertRaisesRegex(ValueError, '27B prompt sets'):
                    manifest_tool.load_manifest(file, 'P2', '/bin/true', '/bin/true', '0')
                config['prompt_sets']['W2']['sha256'] = 'e5480af17428c48e614a0840f30eda53acbea7f9efe869c5173cde5d5818edfd'
                file.write_text(json.dumps(manifest))
                with self.assertRaisesRegex(ValueError, 'prompt set hash mismatch'):
                    manifest_tool.load_manifest(file, 'P2', '/bin/true', '/bin/true', '0')
                h2 = config['prompt_sets']['W2']['sha256']
                config['server_flags'].remove('--no-enable-prefix-caching'); file.write_text(json.dumps(manifest))
                with self.assertRaises(ValueError): manifest_tool.load_manifest(file, 'P2', '/bin/true', '/bin/true', '0')
                config['server_flags'].append('--no-enable-prefix-caching'); config['revision'] = 'a' * 40; file.write_text(json.dumps(manifest))
                with self.assertRaises(ValueError): manifest_tool.load_manifest(file, 'P2', '/bin/true', '/bin/true', '0')

                config['model'] = 'Qwen/Qwen3.8-27B'
                config['revision'] = '1d4bf0f2ff6012fd82039f2fa52739d0dd7c60c0'
                config['server_flags'] = ['--max-model-len', '16384', '--max-num-seqs', '128', '--gpu-memory-utilization', '0.90',
                                          '--default-chat-template-kwargs', '{"enable_thinking": false}', '--no-enable-prefix-caching']
                preset = Path(manifest_tool.__file__).resolve().parents[3] / 'docs/telemetry/examples/all-smi.yaml'
                config.update(all_smi_binary={'path': '/bin/true', 'sha256': binary},
                              all_smi_protoc_crate={'path': str(asset), 'sha256': asset_hash},
                              telemetry_config={'path': str(preset), 'sha256': real_digest(preset)},
                              all_smi_checkout=str(root))
                manifest['lanes'] = {'P1': config}
                for h2 in ('e5480af17428c48e614a0840f30eda53acbea7f9efe869c5173cde5d5818edfd',
                           'eb1ffdab20236c7aaadf63de3c03350076a47cccf796f279aacd551a38d8df8b'):
                    with self.subTest(lane='P1', w2=h2):
                        config['prompt_sets']['W2']['sha256'] = h2; file.write_text(json.dumps(manifest))
                        manifest_tool.load_manifest(file, 'P1', '/bin/true', '/bin/true', '0')


if __name__ == '__main__': unittest.main()
