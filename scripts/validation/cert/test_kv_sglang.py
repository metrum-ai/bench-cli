# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline source-slice and negative controls; never live certification."""
import argparse
import base64
import json
import unittest
import tempfile
from contextlib import ExitStack
from types import SimpleNamespace
from pathlib import Path
from unittest.mock import patch

import case_dispatch
import kv_external_common as common
import kv_sglang_runner as runner

PAGE = (Path(__file__).resolve().parents[3] / 'tests/fixtures/sglang0523-source-slice.prom').read_text()


def page(requests=0, hit=0):
    return (f'# TYPE {runner.HIT} counter\n{runner.HIT}{{cache_source="device",worker="0"}} {hit}\n'
            f'# TYPE {runner.RATE} gauge\n{runner.RATE}{{worker="0"}} 0.5\n'
            f'# TYPE {runner.REQUESTS} counter\n{runner.REQUESTS}{{worker="0",is_streaming="true"}} {requests}\n')


class Checks(unittest.TestCase):
    def test_source_fixture_and_ratio_have_distinct_units(self):
        rows = common.parse_page(PAGE, {runner.HIT: 'counter', runner.RATE: 'gauge'})
        self.assertEqual(4, len(rows))
        self.assertEqual({'device', 'host', 'total'}, {dict(k[1])['cache_source'] for k in rows if k[0] == runner.HIT})
        self.assertEqual({('sglang:cache_hit_rate', (('model_name', 'fixture-model'), ('tp_rank', '0'))): 0.625},
                         {k: v for k, v in rows.items() if k[0] == runner.RATE})

    def test_valid_window_never_claims_occupancy_or_physical_tier(self):
        value = runner.observation(page(), page(2, 64), 2, True)
        self.assertEqual({'device': 64}, value['cache_source_token_deltas'])
        self.assertIsNone(value['occupancy'])
        self.assertEqual('unknown', value['physical_tier'])

    def test_missing_reset_worker_type_duplicate_and_no_reuse_block(self):
        for changed in [page(2, 0), page(3, 64), page(2, 64).replace('worker="0"', 'worker="1"'),
                        page(2, 64).replace('counter', 'gauge'), page(2, 64).replace(' 0.5', ' 1.5'),
                        page(2, 64) + f'{runner.HIT}{{cache_source="device",worker="0"}} 64\n',
                        page(2, 64).replace(f'{runner.HIT}{{cache_source="device",worker="0"}} 64\n', ''),
                        page(2, 64).replace(' 64\n', ' NaN\n'), page(2, 64).replace(' 64\n', ' 0.5\n')]:
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                runner.observation(page(), changed, 2, True)
        with self.assertRaises(ValueError):
            runner.observation(page(1, 64), page(2, 32), 1, True)

    def test_hicache_unavailable_is_not_silently_credited(self):
        with self.assertRaises(ValueError):
            runner.validate_config({'image': runner.IMAGE, 'source_revision': runner.REVISION,
                'model': 'Qwen/Qwen3.8-27B-FP8', 'revision': 'c3a462cc3d38cf351f3b26cbd763469eeb74ef81',
                'server_flags': ['--enable-metrics'], 'owner': 'test', 'evidence_link': 'test', 'kv_case': 'BC-KV-004'})

    def test_dispatch_and_unknown_selection(self):
        with patch.dict('os.environ', {'CERT_CASE_ID': 'BC-KV-005'}), self.assertRaises(SystemExit):
            runner.main()

    def test_full_fake_runner_retains_only_provisional_results_and_unknown_blocks(self):
        for unknown in (False, True):
            with self.subTest(unknown=unknown), tempfile.TemporaryDirectory() as directory, ExitStack() as stack:
                root = Path(directory)
                evidence = root / 'evidence'
                evidence.mkdir()
                (evidence / 'gpu.csv').write_text('fixture-gpu,fixture-driver\n')
                binary = root / 'candidate'
                binary.write_bytes(b'fixture')
                config = {'model': 'fixture', 'port': 8000, 'proxy_port': 8001, 'settle_seconds': 1,
                          'revision': 'a' * 40, 'owner': 'test', 'evidence_link': 'test'}
                manifest = root / 'manifest.json'
                manifest.write_text(json.dumps({'lanes': {'KVS': config}}))
                args = SimpleNamespace(out=root, candidate=binary, baseline=binary, manifest=manifest,
                                       container='fixture', container_owner='owner', start_server=False)
                capture = {'profile': 'sglang0523', 'reference_source_revision': runner.REVISION,
                           'failure': None, 'samples': [{'metric': runner.RATE, 'value': None if unknown else .5,
                           'unknown': 'invalid_value' if unknown else None, 'unit': 'ratio',
                           'role': 'reported_hit_rate', 'expected_family_type': 'gauge'}]}
                cohort = {'rows': [{'native_cache_metric_discovery': capture}], 'requests': [{}, {}],
                          'usage': [{'prompt_tokens_details': {'cached_tokens': 64}}]}
                binding = {'image': runner.CONFIG, 'owner': 'owner', 'restarts': 0}
                stack.enter_context(patch.dict('os.environ', {'CERT_CASE_ID': 'BC-KV-003', 'CERT_SHA': 'a' * 40,
                                                             'CERT_START_UTC': '2026-01-01T00:00:00Z'}))
                stack.enter_context(patch.object(argparse.ArgumentParser, 'parse_args', return_value=args))
                stack.enter_context(patch.object(runner, 'validate_config', return_value={}))
                stack.enter_context(patch.object(runner, 'runtime_source', return_value={'matched': True}))
                stack.enter_context(patch.object(common, 'docker_json', return_value=binding))
                stack.enter_context(patch.object(runner.time, 'sleep'))
                stack.enter_context(patch.object(common, 'run_cohort', return_value=cohort))
                stack.enter_context(patch.object(common, 'fetch', side_effect=[page(), page(), page(2, 64), page(2, 64),
                                                                            page(2, 64), page(2, 64), page(4, 64), page(4, 64)]))
                self.assertEqual(1 if unknown else 0, runner.main())
                record = json.loads((evidence / 'provisional-BC-KV-003.json').read_text())
                self.assertEqual('FAIL' if unknown else 'PASS', record['result'])
                self.assertFalse((root / 'results').exists())

    def test_identity_and_complete_usage_are_required(self):
        import hashlib
        body = b'data: {"usage":{"prompt_tokens":80,"completion_tokens":1,"prompt_tokens_details":{"cached_tokens":64}}}\n\ndata: [DONE]\n\n'
        data = (json.dumps({'schema_version': 'llm.request.v1', 'phase': 'measure', 'error': None}) + '\n'
                + json.dumps({'schema_version': 'llm.summary.v1', 'attempted': 1, 'successes': 1}))
        events = [{'request': 1, 'kind': 'send'}, {'request': 1, 'kind': 'complete'},
                  {'request': 1, 'kind': 'request_identity', 'model': 'fixture', 'streaming': True, 'max_tokens': 32, 'temperature': 0, 'seed': 42, 'thinking': False,
                   'prompt_sha256': hashlib.sha256(b'hello').hexdigest()},
                  *[{'request': 1, 'kind': k, 'bytes_b64': base64.b64encode(body).decode()} for k in ('chunk', 'forwarded_chunk')]]
        trace = '\n'.join(map(json.dumps, events))
        self.assertEqual(64, common.verify_trace(data, trace, ['hello'], 'fixture')[2][0]['prompt_tokens_details']['cached_tokens'])
        for altered in [trace.replace('complete', 'other'), trace.replace('fixture', 'wrong')]:
            with self.assertRaises(ValueError):
                common.verify_trace(data, altered, ['hello'], 'fixture')
        with self.assertRaises(ValueError):
            common.verify_trace(data, trace, ['different'], 'fixture')


if __name__ == '__main__':
    unittest.main()
