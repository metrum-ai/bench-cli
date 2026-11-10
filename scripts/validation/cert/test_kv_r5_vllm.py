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
from types import SimpleNamespace
from unittest.mock import patch

import kv_r5_metrics
import kv_r5_eviction_observer as observer
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
                         'cached_status': 'reported', 'prompt_status': 'reported', 'source': 'openai-chat-usage.v1:/prompt_tokens_details/cached_tokens',
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
    def test_engine_identity_aliases_are_rejected_before_launch(self):
        data = {'model': runner.MODEL, 'model_revision': runner.MODEL_REVISION, 'served_model': runner.MODEL,
                'metrics_port': 9000, 'port': 9000, 'metrics_internal_port': 8000}
        for flag in ('--served_model_name=other', '--served-model-n=other', '--hf_overrides.architectures=Other', '--config=other'):
            with self.subTest(flag=flag), self.assertRaisesRegex(ValueError, 'identity override'):
                runner.validate({**data, 'server_flags': ['--enable-prefix-caching', flag]})
        for flag in ('--max_num_seqs=512', '--max-num-seq=512'):
            with self.subTest(flag=flag), self.assertRaisesRegex(ValueError, 'max-num-seqs 256'):
                runner.validate({**data, 'gpu_class': 'H100',
                                 'server_flags': ['--enable-prefix-caching', '--max-num-seqs', '256', flag]})

    def test_rc_package_cannot_use_stable_runtime_profile(self):
        instance = SimpleNamespace(data={'case': 'BC-KV-002'}, out=Path('/unused'), name='owned-fixture', docker=lambda *args:
            SimpleNamespace(stdout=json.dumps({'version': '0.33.0rc1', 'files': {}})))
        with patch.object(runner.runtime, 'save'), self.assertRaisesRegex(ValueError, 'not stable'):
            runner.preflight(instance)

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

    def test_pressure_cannot_attribute_target_a_eviction_to_a_different_rehit(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            phases = [phase(root, name, count=1, cached=4 if name == 'warm' else 0,
                            offset=100 if name == 'pressure' else 200 if name == 'control' else 9 if name == 'rehit' else 0)
                      for name in ('cold', 'populate', 'warm', 'pressure', 'rehit', 'control')]
            with self.assertRaisesRegex(ValueError, 'target A wire identity'):
                runner.evaluate({'case': 'BC-KV-009', 'target_prefix_sha256': 'a'*64}, phases)

    def test_native_offload_keeps_all_four_conditions_with_builtin_observer(self):
        with tempfile.TemporaryDirectory() as tmp:
            phases = [phase(Path(tmp), name, count=1, offset=100 if name == 'pressure' else 0)
                      for name in ('cold', 'populate', 'pressure', 'rehit')]
            rehit = phases[-1]
            rehit['plan']['native_cpu_load'] = True
            extra = f'# TYPE {runner.LOAD} counter\n{runner.LOAD}{{engine="0"}} '
            rehit['before'] += extra+'0\n'
            rehit['after'] += extra+'8\n'
            path = rehit['directory']/'requests.csv'
            with path.open() as stream:
                rows = list(csv.DictReader(stream))
            rows[0]['ttft_s'] = '0.5'
            with path.open('w') as stream:
                writer = csv.DictWriter(stream, fieldnames=list(rows[0]))
                writer.writeheader()
                writer.writerows(rows)
            data = {'case': 'BC-KV-002', 'target_prefix_sha256': 'a'*64}
            with patch.object(observer, 'verify', return_value={'target_gpu_evicted': True}) as verify:
                runner.evaluate(data, phases)
                verify.assert_called_once_with(data, phases)
                old = rehit['after']; rehit['after'] = rehit['after'].replace(extra+'8', extra+'0')
                with self.assertRaisesRegex(ValueError, 'byte/hit'):
                    runner.evaluate(data, phases)
                rehit['after'] = old
                summary = rehit['directory']/'stdout.json'
                original_summary = summary.read_text()
                rehit['after'] = page(110, 4)+extra+'8\n'
                series = kv_r5_metrics.deltas(rehit['before'], rehit['after'], runner.APC)
                summary.write_text(json.dumps({'config': {'native_cache_phase_analysis': [
                    {'delta': {'window_unknown': None, 'continuity': 'operator_declared',
                               'series': [{**r, 'unknown': None} for r in series]}}]}}))
                with self.assertRaisesRegex(ValueError, 'byte/hit'):
                    runner.evaluate(data, phases)
                rehit['after'] = old
                summary.write_text(original_summary)
                rows[0]['ttft_s'] = '1'
                with path.open('w') as stream:
                    writer = csv.DictWriter(stream, fieldnames=list(rows[0]))
                    writer.writeheader()
                    writer.writerows(rows)
                with self.assertRaisesRegex(ValueError, 'strictly below'):
                    runner.evaluate(data, phases)
            with patch.object(observer, 'verify', side_effect=ValueError('target GPU eviction missing')):
                with self.assertRaisesRegex(ValueError, 'eviction missing'):
                    runner.evaluate(data, phases)

    def test_existing_real_exporter_serialization_is_parseable(self):
        fixture = Path(__file__).resolve().parents[3]/'tests/fixtures/vllm033-logger-synthetic-config.prom'
        values = kv_r5_metrics.snapshot(fixture.read_text(), runner.APC)
        self.assertEqual(len(values), 2)
        self.assertEqual(sorted(values.values()), [40, 100])


if __name__ == '__main__':
    unittest.main()
