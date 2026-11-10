# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Stable KVBM exporter parsing and fail-closed deployment controls."""
import copy
import unittest
from pathlib import Path
from unittest.mock import patch

import kv_r5_dynamo_runner as runner


class DynamoControls(unittest.TestCase):
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
        with patch.object(runner.runtime, 'product_rows', return_value=([{}], [{'prompt_tokens': 8}])):
            with self.assertRaisesRegex(ValueError, 'not observed'):
                runner.evaluate({}, phases)
            phases[1]['after'] = self.page.replace('kvbm_matched_tokens 64', 'kvbm_matched_tokens 96')
            result = runner.evaluate({}, phases)
            self.assertEqual(result['physical_tier'], 'unknown')
            self.assertIsNone(result['bytes_per_block'])


if __name__ == '__main__':
    unittest.main()
