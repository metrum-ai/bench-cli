# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""CPU fakes and actual upstream serialization; never GPU certification evidence."""
import base64
import copy
import hashlib
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import kv_r5_eviction_observer as observer
import kv_r5_proxy


def b64(value):
    return base64.b64encode(value).decode()


def page(count, query=None, running=0, worker='0'):
    labels = f'engine="{worker}",model_name="fixture"'
    return (f'# TYPE {observer.SUCCESS} counter\n'
            + ''.join(f'{observer.SUCCESS}{{{labels},finished_reason="{reason}"}} {count if reason == "stop" else 0}\n'
                      for reason in ('stop', 'length', 'abort', 'error', 'repetition'))
            + f'# TYPE {observer.RUNNING} gauge\n{observer.RUNNING}{{{labels}}} {running}\n'
            + f'# TYPE {observer.WAITING} gauge\n{observer.WAITING}{{{labels}}} 0\n'
            + f'# TYPE {observer.CONFIG} gauge\n{observer.CONFIG}{{engine="{worker}",block_size="2",num_gpu_blocks="16"}} 1\n'
            + '# TYPE vllm:prefix_cache_queries_total counter\n'
            + f'vllm:prefix_cache_queries_total{{{labels}}} {count*4 if query is None else query}\n'
            + '# TYPE vllm:prefix_cache_hits_total counter\n'
            + f'vllm:prefix_cache_hits_total{{{labels}}} 0\n')


class ObserverControls(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        fixture = Path(__file__).resolve().parents[3]/'tests/fixtures/vllm033-kv-events-synthetic.json'
        self.fixture = json.loads(fixture.read_text())
        self.rows = self.fixture['replay']
        self.frozen = {'schema_version': 'vllm033-eviction-plan.v1', 'prefix_token_ids': [1, 2, 3, 4],
                       'target_request': {'model': 'fixture', 'messages': [{'role': 'user', 'content': 'synthetic A'}], 'stream': True},
                       'groups': [{'group_idx': 0, 'kind': 'full_attention', 'block_size': 2},
                                  {'group_idx': 1, 'kind': 'mamba', 'block_size': 4}],
                       'cache_config': {'block_size': 2, 'num_gpu_blocks': 16}}
        self.data = {'case': 'BC-KV-002', 'served_model': 'fixture', 'target_prefix_sha256': observer.token_hash([1, 2, 3, 4]),
                     'image': 'source-pinned-fixture-image', 'model_revision': 'a'*40,
                     'eviction_observer': 'vllm033-kv-events.v1', 'server_flags': [], 'inputs': {}, 'phases': []}
        identity = kv_r5_proxy.identity(json.dumps(self.frozen['target_request']))
        self.request = {key: identity[key] for key in ('messages_sha256', 'options')}
        for name, count in [('cold', 1), ('populate', 1), ('pressure', 2), ('rehit', 1)]:
            requests = [self.request]*count
            if name == 'pressure':
                requests = [{**self.request, 'messages_sha256': 'b'*64}]*count
            self.data['phases'].append({'name': name, 'expected_requests': requests})
        evidence = self.root/'inventory-raw.txt'
        evidence.write_text('synthetic engine group inventory: full attention and Mamba')
        self.data['inputs']['group_inventory_raw'] = self.pin(evidence)
        self.data['inputs']['eviction_observer_module'] = self.pin(Path(observer.__file__).resolve())
        self.write_pin('kv_group_inventory', {'source_revision': observer.REVISION, 'groups': self.frozen['groups'],
                                             'image': self.data['image'], 'model_revision': self.data['model_revision'],
                                             'raw_evidence': [self.pin(evidence)]})
        self.write_pin('eviction_plan', self.frozen)

    def pin(self, path):
        return {'path': str(path), 'sha256': hashlib.sha256(path.read_bytes()).hexdigest()}

    def write_pin(self, name, value):
        path = self.root/(name+'.json')
        path.write_text(json.dumps(value))
        self.data['inputs'][name] = self.pin(path)

    def cut(self, count):
        return self.rows[:count]+[self.rows[-1]]

    def test_actual_source_publisher_serialization_preserves_duplicate_entries(self):
        self.assertEqual(self.fixture['source_revision'], observer.REVISION)
        self.assertEqual(len(observer.decode_replay(self.rows)), 4)
        self.assertEqual(observer.state(self.cut(1), self.frozen)['resident_target_entries'], 3)
        self.assertEqual(observer.state(self.cut(2), self.frozen)['resident_target_entries'], 6)
        self.assertEqual(observer.state(self.cut(3), self.frozen)['resident_target_entries'], 3)
        self.assertEqual(observer.state(self.cut(4), self.frozen)['resident_target_entries'], 0)

    def test_truncation_restarts_gaps_wrong_topic_rank_and_missing_end_block(self):
        mutations = [self.rows[:-1], self.rows[1:], self.rows+[self.rows[-1]], self.rows[:1]*2+[self.rows[-1]]]
        topic, rank, truncated = [copy.deepcopy(self.rows) for _ in range(3)]
        topic[0][1] = b64(b'other')
        raw = base64.b64decode(rank[0][3]); self.assertEqual(raw[-1], 0)
        rank[0][3] = b64(raw[:-1]+b'\x01')
        truncated[0][3] = b64(raw[:-2])
        for rows in mutations+[topic, rank, truncated]:
            with self.subTest(rows=rows[0][:3]), self.assertRaises(ValueError):
                observer.decode_replay(rows)

    def test_unproven_hash_spec_group_and_missing_full_attention_block(self):
        for groups in ([self.frozen['groups'][0]],
                       [{**self.frozen['groups'][0], 'block_size': 1}, self.frozen['groups'][1]],
                       [self.frozen['groups'][0], {**self.frozen['groups'][1], 'kind': 'unknown'}]):
            with self.subTest(groups=groups), self.assertRaises(ValueError):
                observer.state(self.rows, {**self.frozen, 'groups': groups})
        removed = copy.deepcopy(self.rows[2]); removed[2] = b64(bytes(8))
        with self.assertRaisesRegex(ValueError, 'without prior store'):
            observer.state([removed, self.rows[-1]], self.frozen)
        conflict = copy.deepcopy(self.rows)
        raw = base64.b64decode(conflict[1][3])
        # A second source-serialized store reuses the same hash with other tokens.
        conflict[1][3] = b64(raw.replace(b'\x94\x01\x02\x03\x04', b'\x94\x09\x02\x03\x04'))
        with self.assertRaisesRegex(ValueError, 'conflicting opaque hash'):
            observer.state(conflict, self.frozen)
        self.assertNotEqual(observer.opaque(1), observer.opaque(b'\x01'))

    def test_decoder_rejects_duplicate_keys_extensions_and_trailing_bytes(self):
        for raw in (b'\x82\xa1x\x01\xa1x\x02', b'\xd4\x00\x00', b'\xc0\xc0'):
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                observer.unpack(raw)

    def test_masked_empty_store_is_not_a_residency_change(self):
        # Decode real serialized events, then inject a source-valid masked
        # range through a fake replay boundary to test interpretation only.
        batch = observer.decode_replay(self.cut(1))[0]
        empty = {**batch[-1], 'block_hashes': []}
        with patch.object(observer, 'decode_replay', return_value=[batch+[empty]]):
            self.assertEqual(observer.state([], self.frozen)['resident_target_entries'], 3)
        for changed in ({'type': 'AllBlocksCleared'}, {**batch[0], 'lora_name': 'adapter'},
                        {**batch[0], 'lookahead_token_ids': [[9], [9]]},
                        {**batch[0], 'extra_keys': [['salt'], None]}):
            with self.subTest(event=changed), patch.object(observer, 'decode_replay', return_value=[batch+[changed]]):
                with self.assertRaises(ValueError):
                    observer.state([], self.frozen)

    def test_partial_mamba_alias_is_tracked_in_addition_to_aligned_endpoint(self):
        batch = observer.decode_replay(self.cut(1))[0]
        alias = {**batch[-1], 'block_hashes': [batch[0]['block_hashes'][0]], 'block_size': 2, 'token_ids': [1, 2]}
        with patch.object(observer, 'decode_replay', return_value=[batch+[alias]]):
            self.assertEqual(observer.state([], self.frozen)['resident_target_entries'], 4)

    def test_plan_requires_pinned_inventory_target_and_incremental_reporting(self):
        observer.validate(self.data)
        for bad in ('full', True, None):
            changed = copy.deepcopy(self.data)
            changed['phases'][0]['expected_requests'][0]['options']['kv_cache_report_mode'] = bad
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                observer.validate(changed)
        for flag in ('--kv-events-config=x', '--kv-transfer-config', '-tp', '--speculative-config'):
            with self.subTest(flag=flag), self.assertRaisesRegex(ValueError, 'override'):
                observer.validate({**self.data, 'server_flags': [flag]})
        with self.assertRaisesRegex(ValueError, 'reset'):
            observer.validate({**self.data, 'phases': [{**self.data['phases'][0], 'reset_prefix_cache': True}]})
        changed = copy.deepcopy(self.frozen); changed['prefix_token_ids'][-1] = 9
        self.write_pin('eviction_plan', changed)
        with self.assertRaisesRegex(ValueError, 'token identity'):
            observer.plan(self.data)

    def test_counter_scope_geometry_and_resets_fail_closed(self):
        values = observer.counter_snapshot(page(2), self.frozen, 'fixture')
        self.assertEqual(observer.completion_delta(values, observer.counter_snapshot(page(3), self.frozen, 'fixture')), 1)
        for bad in (page(2, running=1), page(2, worker='1'), page(2).replace('num_gpu_blocks="16"', 'num_gpu_blocks="32"')):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                observer.counter_snapshot(bad, self.frozen, 'fixture')
        with self.assertRaisesRegex(ValueError, 'reset'):
            observer.completion_delta(values, {**values, 'stop': 1})

    def write_capture(self, directory, rows, count, before=False, first=False):
        directory.mkdir()
        def row(method, path, value, request=None):
            return {'method': method, 'path': path, 'request': request, 'status': 200,
                    'body_b64': b64(json.dumps(value).encode())}
        controls = [row('POST', '/pause?mode=wait&clear_cache=false', {'status': 'paused'}),
                    row('GET', '/is_paused', {'is_paused': True})]
        if first:
            request = {'model': 'fixture', 'messages': self.frozen['target_request']['messages'],
                       'add_special_tokens': False, 'add_generation_prompt': True,
                       'continue_final_message': False, 'chat_template_kwargs': None}
            controls.append(row('POST', '/tokenize', {'tokens': [1, 2, 3, 4, 5]}, request))
        if before:
            controls.append(row('POST', '/resume', {'status': 'resumed'}))
        (directory/'control.jsonl').write_text('\n'.join(map(json.dumps, controls)))
        (directory/'metrics-000.prom').write_text(page(count))
        for index in (0, 1):
            observer.runtime.save(directory/f'replay-{index:03}.json', rows)
        observer.runtime.save(directory/'snapshot.json', {'metrics_attempt': 0, 'replay_attempt': 1})

    def evidence(self):
        phases, previous, count = [], self.cut(0), 0
        epoch = [{'pid': 10, 'start_ticks': 20, 'cmdline_sha256': 'a'*64}]
        observer.runtime.save(self.root/'worker-epoch.json', epoch)
        for index, (p, tail) in enumerate(zip(self.data['phases'], (1, 2, 4, 4))):
            directory = self.root/p['name']; directory.mkdir()
            self.write_capture(directory/'observer-before', previous, count, before=True, first=index == 0)
            count += len(p['expected_requests'])
            previous = self.cut(tail)
            self.write_capture(directory/'observer-after', previous, count)
            observer.runtime.save(directory/'worker-epoch.json', epoch)
            phases.append({'plan': p, 'directory': directory})
        return phases

    def test_recomputed_complete_exclusive_eviction_proof_and_competing_request(self):
        phases = self.evidence()
        proof = observer.verify(self.data, phases)
        self.assertTrue(proof['target_gpu_evicted'])
        self.assertEqual(proof['gpu_lookup_entries_before'], 6)
        self.assertEqual(proof['gpu_lookup_entries_after'], 0)
        path = phases[-1]['directory']/'observer-after/metrics-000.prom'
        path.write_text(page(6))
        with self.assertRaisesRegex(ValueError, 'competing/missing'):
            observer.verify(self.data, phases)

    def test_single_removal_cannot_prove_eviction_with_duplicate_blocks(self):
        phases = self.evidence()
        for phase in phases[2:]:
            for side in ('before', 'after'):
                if phase['plan']['name'] == 'pressure' and side == 'before':
                    continue
                for i in (0, 1):
                    path = phase['directory']/('observer-'+side)/f'replay-{i:03}.json'
                    path.write_text(json.dumps(self.cut(3)))
        with self.assertRaisesRegex(ValueError, 'not fully evicted'):
            observer.verify(self.data, phases)

    def test_epoch_change_and_unsettled_replay_pair_block(self):
        phases = self.evidence()
        worker = phases[-1]['directory']/'worker-epoch.json'
        original = worker.read_text(); worker.write_text('[{"pid":999}]')
        with self.assertRaisesRegex(ValueError, 'continuity'):
            observer.verify(self.data, phases)
        worker.write_text(original)
        path = phases[-1]['directory']/'observer-after/replay-000.json'
        path.write_text(json.dumps(self.cut(3)))
        with self.assertRaisesRegex(ValueError, 'not stable'):
            observer.verify(self.data, phases)

    def test_live_capture_uses_drain_api_and_bounded_stock_replay_client(self):
        directory = self.root/'capture'
        instance = SimpleNamespace(name='owned-fixture', data=self.data, deadline=observer.time.monotonic()+60,
                                   get=lambda *args: page(0),
                                   docker=lambda *args, **kwargs: SimpleNamespace(stdout=json.dumps(self.cut(0))))
        instance.data['port'] = 9999
        calls = []
        def control(instance, directory, method, path, body=None):
            calls.append((method, path))
            return {'status': 'paused'} if method == 'POST' else {'is_paused': True}
        with patch.object(observer, 'control', side_effect=control):
            rows, counts = observer.capture(instance, directory)
        self.assertEqual(rows, self.cut(0))
        self.assertEqual(calls, [('POST', '/pause?mode=wait&clear_cache=false'), ('GET', '/is_paused')])
        self.assertEqual(sum(counts.values()), 0)
        self.assertTrue((directory/'replay-001.json').is_file())


if __name__ == '__main__':
    unittest.main()
