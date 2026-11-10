# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Stable KVBM exporter parsing and fail-closed deployment controls."""
import copy
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import kv_r5_dynamo_runner as runner
import kv_r5_proxy as proxy


class DynamoControls(unittest.TestCase):
    def test_rc_backend_is_rejected_even_in_a_claimed_stable_image(self):
        instance = SimpleNamespace(out=Path('/unused'), name='owned-fixture', docker=lambda *args:
            SimpleNamespace(stdout='{"backend_version":"0.31.0rc1"}'))
        with patch.object(runner.runtime, 'save'), self.assertRaisesRegex(ValueError, 'not stable'):
            runner.preflight(instance)

    def setUp(self):
        self.page = (Path(__file__).resolve().parents[3]/'tests/fixtures/dynamo162-registry-synthetic.prom').read_text()

    def test_real_registry_values_keep_blocks_tokens_and_rates_distinct(self):
        rows = runner.metrics.deltas(self.page, self.page.replace('kvbm_matched_tokens 64', 'kvbm_matched_tokens 96'), runner.SPECS)
        self.assertEqual(runner.metrics.single(rows, 'kvbm_matched_tokens'), 32)
        rate = next(r for r in rows if r['metric'] == 'kvbm_host_cache_hit_rate')
        self.assertIsNone(rate['delta'])
        self.assertEqual(rate['unit'], 'ratio')

    def test_missing_gauge_reset_and_invalid_rate_block(self):
        for page in [self.page.replace('kvbm_host_cache_hit_rate 0.5', ''),
                     self.page.replace('kvbm_host_cache_hit_rate 0.5', 'kvbm_host_cache_hit_rate 1.1'),
                     self.page.replace('kvbm_matched_tokens 64', 'kvbm_matched_tokens 63')]:
            with self.assertRaises(ValueError):
                runner.metrics.deltas(self.page, page, runner.SPECS)

    def test_no_reuse_cannot_pass(self):
        phases = [{'plan': {'name': name, 'expected_requests': [
            {'messages_sha256': name if name == 'control' else 'same', 'warmup': False}]},
                   'before': self.page, 'after': self.page} for name in ('populate', 'reuse', 'control')]
        with tempfile.TemporaryDirectory() as tmp, patch.object(runner.runtime, 'product_rows', return_value=([{}], [{'prompt_tokens': 8}])):
            root = Path(tmp)
            runner.runtime.save(root/'gpu.json', {'observed': 'GPU-fixture,H100,81920,fixture'})
            for phase in phases:
                directory = root/phase['plan']['name']
                directory.mkdir()
                phase['directory'] = directory
                (directory/'trace.jsonl').write_text(json.dumps({'kind': 'request_identity', 'text_only': True}))
            first = phases[0]['directory']
            (first/'memory-profile.log').write_text('Available KV cache memory: 30.0 GiB')
            runner.runtime.save(first/'headroom.json', runner.headroom('Available KV cache memory: 30.0 GiB', 81920))
            with self.assertRaisesRegex(ValueError, 'not observed'):
                runner.evaluate({}, phases)
            phases[1]['after'] = self.page.replace('kvbm_matched_tokens 64', 'kvbm_matched_tokens 96')
            result = runner.evaluate({}, phases)
            self.assertEqual(result['physical_tier'], 'unknown')
            self.assertIsNone(result['bytes_per_block'])
            self.assertEqual(result['model'], runner.MODEL)
            (first/'trace.jsonl').write_text(json.dumps({'kind': 'request_identity', 'text_only': False}))
            with self.assertRaisesRegex(ValueError, 'text-only'):
                runner.evaluate({}, phases)

    def test_headroom_is_measured_and_rounding_is_conservative(self):
        self.assertGreater(runner.headroom('Available KV cache memory: 30.0 GiB', 81920)['conservative_headroom_fraction'], .25)
        for text in ('', 'Available KV cache memory: 20.0 GiB',
                     'Available KV cache memory: 19.99 GiB',
                     'Available KV cache memory: 90.0 GiB',
                     'Available KV cache memory: 30.0 GiB\nAvailable KV cache memory: 30.0 GiB'):
            with self.subTest(text=text), self.assertRaises(ValueError):
                runner.headroom(text, 81920)

    def test_media_and_embedding_messages_are_not_text_only(self):
        for content in ([{'type': 'image_url', 'image_url': {'url': 'fixture'}}], None, {'prompt_embeds': 'fixture'}):
            actual = proxy.identity(json.dumps({'messages': [{'role': 'user', 'content': content}]}))
            self.assertIs(actual['text_only'], False)
        self.assertIs(proxy.identity('{"messages":[{"role":"user","content":"hello"}]}')['text_only'], True)

    def test_exact_model_and_text_only_server_cannot_be_overridden(self):
        data = {'model': runner.MODEL, 'model_revision': runner.MODEL_REVISION,
                'served_model': 'model', 'cpu_cache_gb': 4, 'server_flags': ['--enable-prefix-caching']}
        argv = runner.server(data)
        self.assertIn('--language-model-only', argv)
        self.assertEqual(argv[argv.index('--tensor-parallel-size')+1], '1')
        data.update(metrics_internal_port=6880, metrics_port=9001, port=9000, proxy_port=9002,
                    phases=[{'name': name} for name in ('populate', 'reuse', 'control')],
                    model_selection_reason=runner.MODEL_REASON)
        for key, bad in [('model', 'Qwen/Qwen3-8B'), ('model_revision', 'a'*40), ('model_selection_reason', 'unrecorded')]:
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, 'approved'):
                runner.validate({**data, key: bad})
        data.update(model_snapshot='/fixture', inputs={'kvbm_build_provenance': {}})
        proof = {'source_revision': runner.REVISION, 'image': runner.IMAGE,
                 'independent_build_evidence': [{}], 'extension_sha256': {'fixture.so': 'fixture'}}
        def load(path):
            return {'architectures': [runner.ARCHITECTURE], 'model_type': 'qwen3_vl'} if str(path).endswith('config.json') else proof
        with patch.object(runner.runtime.manifest_tool, 'sha256_file', return_value='2329a2c9022d94906d5af9bafe93ca97cba0eedb5426fc42ca64f2bfcd65e949'), patch.object(runner.runtime, 'pin'), patch.object(runner.runtime, 'load', side_effect=load):
            runner.validate(data)
            for flag in ('--language-model-only=false', '--enable-mm-embeds', '--kv-cache-memory-bytes=1', '--hf-overrides', '-tp'):
                with self.subTest(flag=flag), self.assertRaisesRegex(ValueError, 'override'):
                    runner.validate({**data, 'server_flags': ['--enable-prefix-caching', flag]})


if __name__ == '__main__':
    unittest.main()
