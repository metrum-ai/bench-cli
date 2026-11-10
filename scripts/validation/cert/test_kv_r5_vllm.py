# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline evaluator controls; no simulated outcome enters release intake."""
import base64
import copy
import csv
import json
import tempfile
import unittest
from pathlib import Path

import kv_r5_metrics
import kv_r5_vllm_runner as runner


def page(query, hit, worker='0'):
    return (f'# TYPE {runner.QUERY} counter\n{runner.QUERY}{{engine="{worker}"}} {query}\n'
            f'# TYPE {runner.HIT} counter\n{runner.HIT}{{engine="{worker}"}} {hit}\n')


def phase(root, name, count=16, cached=0, offset=0):
    directory = root/name
    directory.mkdir()
    expected, rows, events = [], [], []
    for i in range(count):
        request = {'messages_sha256': f'{i+offset:064x}', 'options': {'stream': True},
                   'warmup': False, 'session_id': None, 'turn': None}
        expected.append(request)
        usage = {'prompt_tokens': 10, 'completion_tokens': 1,
                 'prompt_tokens_details': {'cached_tokens': cached}}
        body = ('data: '+json.dumps({'usage': usage})+'\n\ndata: [DONE]\n\n').encode()
        events += [{'request': i+1, 'kind': 'request_identity', **request},
                   {'request': i+1, 'kind': 'headers', 'status': 200},
                   {'request': i+1, 'kind': 'chunk', 'bytes_b64': base64.b64encode(body).decode()},
                   {'request': i+1, 'kind': 'complete'}]
        rows.append({'seq': i+1, 'success': 'true', 'valid': 'true', 'error': '', 'warmup': 'false',
                     'session_id': '', 'turn': '', 'input_tokens': 10, 'output_tokens': 1,
                     'ttft_source': 'stream', 'ttft_s': 1,
                     'cache_evidence': json.dumps({'cached_tokens': cached, 'prompt_tokens': 10,
                         'cached_status': 'reported', 'source': 'openai-chat-usage.v1:/prompt_tokens_details/cached_tokens',
                         'units': 'tokens', 'invalid_report_seen': False, 'conflicting_reports': False})})
    (directory/'trace.jsonl').write_text('\n'.join(map(json.dumps, events)))
    with (directory/'requests.csv').open('w') as stream:
        writer = csv.DictWriter(stream, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)
    before, after = page(100, 0), page(100+10*count, cached*count)
    series = kv_r5_metrics.deltas(before, after, runner.APC)
    (directory/'stdout.json').write_text(json.dumps({'config': {'native_cache_phase_analysis': [
        {'delta': {'window_unknown': None, 'continuity': 'operator_declared',
                   'series': [{**row, 'unknown': None} for row in series]}}]}}))
    return {'directory': directory, 'before': before, 'after': after,
            'plan': {'name': name, 'expected_requests': expected}}


class VllmControls(unittest.TestCase):
    def test_apc_reconciles_product_wire_and_independent_snapshots(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            phases = [phase(root, 'cold'), phase(root, 'warm', cached=4), phase(root, 'unique', offset=100)]
            result = runner.evaluate({'case': 'BC-KV-001'}, phases)
            self.assertEqual(result['phases']['warm']['cohorts']['partial']['n'], 16)
            self.assertIsNone(result['speedup_threshold'])
            self.assertEqual(result['physical_cache_state'], 'unknown')
            phases[1]['after'] = page(260, 63)
            with self.assertRaisesRegex(ValueError, 'differs from independent'):
                runner.evaluate({'case': 'BC-KV-001'}, phases)

    def test_no_reuse_and_incomplete_cohort_block(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            phases = [phase(root, 'cold'), phase(root, 'warm'), phase(root, 'unique', offset=100)]
            with self.assertRaisesRegex(ValueError, 'positive reuse'):
                runner.evaluate({'case': 'BC-KV-001'}, phases)
            rows, usage = runner.runtime.product_rows(phases[0])
            del usage[0]['prompt_tokens_details']
            with self.assertRaisesRegex(ValueError, 'unavailable'):
                runner.cohort(rows, usage)

    def test_duplicate_sessions_missing_warmup_and_missing_reset_block(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = phase(Path(tmp), 'stage')
            with self.assertRaisesRegex(ValueError, 'cold reset'):
                runner.evaluate({'case': 'BC-KV-008'}, [p])
            p['plan']['reset_prefix_cache'] = True
            (p['directory']/'reset.json').write_text('{}')
            with self.assertRaisesRegex(ValueError, 'dedicated session warmup'):
                runner.evaluate({'case': 'BC-KV-008'}, [p])

    def test_offload_and_pressure_require_independent_raw_eviction_evidence(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = phase(Path(tmp), 'rehit', count=1)
            with self.assertRaises(OSError):
                runner.audit(p, 'a'*64)
            proof = {'schema_version': 'kv-exclusive-window.v1', 'phase': 'rehit',
                     'prefix_sha256': 'a'*64, 'raw_event_paths': ['missing.jsonl'],
                     **dict.fromkeys(('same_epoch', 'exclusive_requests', 'exclusive_transfers',
                                     'async_work_settled', 'target_gpu_evicted'), True)}
            (p['directory']/'independent-audit.json').write_text(json.dumps(proof))
            with self.assertRaisesRegex(ValueError, 'raw event evidence absent'):
                runner.audit(p, 'a'*64)
            (p['directory']/'missing.jsonl').write_text('')
            with self.assertRaisesRegex(ValueError, 'stream is empty'):
                runner.audit(p, 'a'*64)
            (p['directory']/'missing.jsonl').write_text(json.dumps({'cached_hit_rate': .9})+'\n')
            with self.assertRaisesRegex(ValueError, 'allocator eviction observation absent'):
                runner.audit(p, 'a'*64)

    def test_existing_real_exporter_serialization_is_parseable(self):
        fixture = Path(__file__).resolve().parents[3]/'tests/fixtures/vllm033-logger-synthetic-config.prom'
        values = kv_r5_metrics.snapshot(fixture.read_text(), runner.APC)
        self.assertEqual(len(values), 2)
        self.assertEqual(sorted(values.values()), [40, 100])


if __name__ == '__main__':
    unittest.main()
