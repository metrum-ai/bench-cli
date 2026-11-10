# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline LMCache source-slice, exclusivity and provisional-result controls."""
import argparse
from contextlib import ExitStack
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import kv_external_common as common
import kv_lmcache_runner as runner

PAGE = (Path(__file__).resolve().parents[3] / 'tests/fixtures/lmcache057-mp-source-slice.prom').read_text()


def page(requests=0, query=0, hit=0, stored=0, loaded=0):
    return ''.join(f'# TYPE {n} counter\n{n}{{model_name="fixture",worker_id="0"}} {v}\n'
                   for n, v in [(runner.QUERY, query), (runner.HIT, hit), (runner.STORED, stored), (runner.LOADED, loaded)]) + (
        f'# TYPE {runner.REQUESTS} counter\n{runner.REQUESTS}{{model_name="fixture",engine="0",finished_reason="length"}} {requests}\n')


class Checks(unittest.TestCase):
    def test_mp_source_serialization_and_no_legacy_aliases(self):
        specs = {n: 'counter' for n in (runner.QUERY, runner.HIT, runner.STORED, runner.LOADED)}
        rows = common.parse_page(PAGE, specs)
        self.assertEqual([1568, 784, 2, 2], [next(v for (name, _), v in rows.items() if name == n) for n in specs])
        with self.assertRaises(ValueError):
            common.parse_page(PAGE.replace('lmcache_mp_', 'lmcache:'), specs)

    def test_same_series_tokens_and_chunks_are_not_resident_occupancy(self):
        value = runner.observation(page(), page(2, 128, 64, 2, 2), 2, page(), page(2))
        self.assertEqual(2, value['stored_chunks'])
        self.assertEqual(64, value['hit_tokens'])
        self.assertIsNone(value['resident_occupancy'])
        self.assertEqual('unknown', value['physical_tier'])

    def test_missing_reset_duplicate_wrong_worker_and_population_block(self):
        after = page(2, 128, 64, 256)
        for changed in [after.replace('counter', 'gauge'), after.replace('worker_id="0"', 'worker_id="1"'),
                        after.replace(' 128\n', ' NaN\n'), after.replace(' 128\n', ' 0.5\n'),
                        after.replace(' 64\n', ' 129\n'), page(3, 128, 64, 256),
                        after + f'{runner.HIT}{{model_name="fixture",worker_id="0"}} 64\n',
                        after.replace('lmcache_mp_', 'lmcache:')]:
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                runner.observation(page(), changed, 2, page(), changed)
        with self.assertRaises(ValueError):
            runner.observation(page(1, 64, 64, 256), page(2, 128, 32, 128), 1, page(1), page(2))

    def test_unknown_selection_and_mode_rejected_before_launch(self):
        with patch.dict('os.environ', {'CERT_CASE_ID': 'BC-KV-003'}), self.assertRaises(SystemExit):
            runner.main()
        with self.assertRaisesRegex(ValueError, 'MP'):
            runner.validate_config({'image': runner.IMAGE, 'source_revision': runner.REVISION, 'connector_mode': 'inprocess'})

    def test_full_fake_requires_store_reuse_and_complete_source_profile(self):
        for mode in ('valid', 'no-store', 'no-reuse', 'unknown', 'wrong-source', 'epoch-change'):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory, ExitStack() as stack:
                root = Path(directory)
                evidence = root / 'evidence'
                evidence.mkdir()
                (evidence / 'models.json').write_text('{"data":[{"id":"fixture"}]}')
                (evidence / 'gpu.csv').write_text('fixture,fixture-driver\n')
                binary = root / 'candidate'
                binary.write_bytes(b'fixture')
                config = {'model': 'fixture', 'port': 8000, 'proxy_port': 8001, 'metrics_port': 8002, 'settle_seconds': 1,
                          'revision': 'a' * 40, 'owner': 'test', 'evidence_link': 'test'}
                manifest = root / 'manifest.json'
                manifest.write_text(json.dumps({'lanes': {'KVL': config}}))
                args = SimpleNamespace(out=root, candidate=binary, baseline=binary, manifest=manifest,
                                       container='fixture', container_owner='owner', start_server=False)
                capture = {'profile': 'lmcache057_mp', 'reference_source_revision': 'wrong' if mode == 'wrong-source' else runner.REVISION,
                           'failure': None, 'samples': [{'metric': n, 'value': None if mode == 'unknown' else 0,
                           'unknown': 'invalid_value' if mode == 'unknown' else None, 'unit': 'tokens' if n in (runner.QUERY, runner.HIT) else 'chunks',
                           'expected_family_type': 'counter'} for n in (runner.QUERY, runner.HIT, runner.STORED, runner.LOADED)]}
                cohort = {'rows': [{'native_cache_metric_discovery': capture}], 'requests': [{}, {}],
                          'usage': [{'prompt_tokens': 128}]}
                initial = page()
                seed = page(2, 128, 0, 0 if mode == 'no-store' else 256)
                reuse = page(4, 256, 0 if mode == 'no-reuse' else 64, 256, 0 if mode == 'no-reuse' else 2)
                unique = page(6, 384, 64, 384, 2)
                binding = {'image': runner.CONFIG, 'owner': 'owner', 'restarts': 0}
                stack.enter_context(patch.dict('os.environ', {'CERT_CASE_ID': 'BC-KV-005', 'CERT_SHA': 'a' * 40,
                                                             'CERT_START_UTC': '2026-01-01T00:00:00Z'}))
                stack.enter_context(patch.object(argparse.ArgumentParser, 'parse_args', return_value=args))
                stack.enter_context(patch.object(runner, 'validate_config', return_value={'max_prompt_tokens': 1024}))
                stack.enter_context(patch.object(runner, 'runtime_source', return_value={'matched': True}))
                stack.enter_context(patch.object(common, 'docker_json', side_effect=[binding, {**binding, 'restarts': 1}] if mode == 'epoch-change' else None,
                                                 return_value=binding))
                stack.enter_context(patch.object(runner.time, 'sleep'))
                stack.enter_context(patch.object(runner, 'reset_native'))
                stack.enter_context(patch.object(common, 'run_cohort', return_value=cohort))
                stack.enter_context(patch.object(common, 'fetch', side_effect=[initial, initial, initial, initial, seed, seed, seed, seed,
                    seed, seed, seed, seed, reuse, reuse, reuse, reuse, reuse, reuse, reuse, reuse, unique, unique, unique, unique]))
                self.assertEqual(0 if mode == 'valid' else 1, runner.main())
                record = json.loads((evidence / 'provisional-BC-KV-005.json').read_text())
                self.assertEqual('PASS' if mode == 'valid' else 'FAIL', record['result'])
                self.assertFalse((root / 'results').exists())


if __name__ == '__main__':
    unittest.main()
