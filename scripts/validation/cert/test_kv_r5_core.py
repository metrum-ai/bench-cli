# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Negative controls for r5 wire/counter intake; never GPU evidence."""
import base64
import json
import tempfile
import time
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import kv_r5_metrics as metrics
import kv_r5_proxy as proxy
import kv_r5_runtime as runtime


class MetricControls(unittest.TestCase):
    def setUp(self):
        self.specs = {'hits_total': ('counter', 'tokens'), 'rate': ('gauge', 'ratio')}
        self.page = '# TYPE hits counter\nhits_total{worker="0"} 4\n# TYPE rate gauge\nrate 0.5\n'

    def test_counter_and_gauge_keep_units(self):
        rows = metrics.deltas(self.page, self.page.replace('} 4', '} 8'), self.specs)
        self.assertEqual(metrics.single(rows, 'hits_total'), 4)
        self.assertIsNone(rows[1]['delta'])
        self.assertEqual(rows[1]['unit'], 'ratio')

    def test_missing_reset_label_change_and_precision_are_rejected(self):
        for after in [self.page.replace('} 4', '} 3'), self.page.replace('worker="0"', 'worker="1"'),
                      self.page.replace('} 4', '} 9007199254740994'),
                      self.page.replace('} 4', '} 9007199254740993'),
                      self.page.replace('} 4', '} 9007199254740992.1'),
                      self.page.replace('hits_total{worker="0"} 4\n', ''),
                      self.page.replace('rate 0.5', 'rate NaN'), self.page.replace('rate 0.5', 'rate 1.1'),
                      self.page.replace('hits_total{worker="0"} 4', 'hits_total{worker="0"} 4.5'),
                      self.page+'hits_total{worker="0"} 4\n',
                      self.page+'# TYPE hits gauge\n', self.page.replace('# TYPE hits counter\n', '')]:
            with self.subTest(after=after), self.assertRaises(ValueError):
                metrics.deltas(self.page, after, self.specs)

    def test_replica_counters_cannot_be_summed(self):
        page = self.page+'hits_total{worker="1"} 2\n'
        with self.assertRaises(ValueError):
            metrics.single(metrics.deltas(page, page, self.specs), 'hits_total')


class WireControls(unittest.TestCase):
    def test_frozen_warmup_and_turn_boundaries_require_typed_values(self):
        good = {'messages_sha256': 'a'*64, 'options': {'model': 'fixture', 'stream': True},
                'warmup': False, 'session_id': None, 'turn': None}
        runtime.expected_request(good, 'fixture')
        for changes in ({'warmup': 'false'}, {'turn': True}, {'messages_sha256': 'a'},
                        {'options': {'model': 'other', 'stream': True}}):
            with self.assertRaises(ValueError):
                runtime.expected_request({**good, **changes}, 'fixture')

    def test_entire_conversation_is_hashed(self):
        a = {'messages': [{'role': 'user', 'content': 'A'}, {'role': 'assistant', 'content': 'B'}], 'stream': True}
        b = {**a, 'messages': list(reversed(a['messages']))}
        self.assertNotEqual(proxy.identity(json.dumps(a).encode()), proxy.identity(json.dumps(b).encode()))

    def test_missing_usage_and_reordered_requests_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            expected = {'messages_sha256': 'a'*64, 'options': {'stream': True}}
            events = [{'request': 1, 'kind': 'request_identity', **expected},
                      {'request': 1, 'kind': 'chunk', 'bytes_b64': base64.b64encode(b'data: [DONE]\n\n').decode()},
                      {'request': 1, 'kind': 'complete'}]
            (directory/'trace.jsonl').write_text('\n'.join(map(json.dumps, events)))
            phase = {'directory': directory, 'plan': {'expected_requests': [expected]}}
            with self.assertRaises(ValueError):
                runtime.wire(phase)
            phase['plan']['expected_requests'][0] = {**expected, 'messages_sha256': 'b'*64}
            with self.assertRaisesRegex(ValueError, 'wire order'):
                runtime.wire(phase)


class LifecycleControls(unittest.TestCase):
    def test_engine_parser_aliases_cannot_override_controlled_options(self):
        forbidden = {'--kv-transfer-config', '--language-model-only', '--kv-cache-memory-bytes', '--served-model-name', '-tp'}
        for value in ('--kv_transfer_config.kv_connector=Other', '--kv-transfer-config+',
                      '--no_language_model_only', '--kv_cache_memory_bytes=1',
                      '--kv-cache-memory-b=1', '--served_model_name+=other', '-tp=2'):
            with self.subTest(value=value):
                self.assertTrue(runtime.server_overrides([value], forbidden))
        self.assertFalse(runtime.server_overrides(['--max-model-len', '4096', '--gpu-memory-utilization=.9'], forbidden))

    def test_unlisted_weights_cannot_hide_behind_a_pinned_model_config(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            config, weights = root/'config.json', root/'model.safetensors'
            config.write_text('{}')
            weights.write_bytes(b'synthetic weight bytes')
            data = {'model_snapshot': str(root), 'model_files': [{'path': str(config)}]}
            with self.assertRaisesRegex(ValueError, 'complete staged snapshot'):
                runtime.model_lock(data)
            data['model_files'].append({'path': str(weights)})
            runtime.model_lock(data)
            (root/'unfrozen-link').symlink_to(weights)
            with self.assertRaisesRegex(ValueError, 'symlinks'):
                runtime.model_lock(data)

    def test_owner_mismatch_prevents_removal(self):
        with tempfile.TemporaryDirectory() as tmp:
            data = {'timeout_seconds': 300, 'tools': {'docker': {'path': '/frozen/docker'}}}
            instance = runtime.Runtime(data, tmp, None)
            instance.started = True
            foreign = SimpleNamespace(stdout=json.dumps([{'Config': {'Labels': {'metrum.cert.owner': 'someone-else'}}}]), stderr='')
            with patch.object(runtime.subprocess, 'run', return_value=foreign) as call:
                with self.assertRaisesRegex(ValueError, 'ownership mismatch'):
                    instance.cleanup()
                self.assertEqual(len(call.call_args_list), 1)
                self.assertEqual(call.call_args.args[0][1], 'inspect')

    def test_total_envelope_reserves_cleanup_and_includes_setup(self):
        with patch.object(time, 'monotonic', return_value=1040.0):
            started = time.monotonic()-40
            instance = runtime.Runtime({'timeout_seconds': 300}, '/unused', None, budget_start=started)
            self.assertEqual(instance.total_deadline, started+300)
            self.assertEqual(instance.deadline, started+180)
            self.assertEqual(instance.deadline-time.monotonic(), 140)


if __name__ == '__main__':
    unittest.main()
