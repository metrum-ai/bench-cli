# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Stable collector fixture and memory-fit negative controls; CPU only."""
import math
import unittest
from pathlib import Path

import kv_r5_trtllm_runner as runner


class StableCollectorControls(unittest.TestCase):
    def setUp(self):
        self.page = (Path(__file__).resolve().parents[3]/'tests/fixtures/trtllm121-collector-synthetic.prom').read_text()
        lines = []
        for line in self.page.splitlines():
            match = runner.SAMPLE.fullmatch(line)
            if match and not line.startswith('#'):
                line = line[:match.start(3)]+str(float(match[3])*2)
            lines.append(line)
        self.after = '\n'.join(lines)

    def test_all_real_collector_families_reconcile_without_cached_token_alias(self):
        delta = runner.compare(self.page, self.after, 1)
        self.assertTrue(any(r['unit'] == 'seconds' and r['delta'] == .1 for r in delta))
        self.assertFalse(any('cached' in r['metric'] for r in delta))
        self.assertIsNone(runner.UNAVAILABLE['value'])
        self.assertEqual(runner.UNAVAILABLE['reason'], 'not exported by 1.2.1')

    def test_missing_family_partial_histogram_reset_and_wrong_request_count_block(self):
        for page in [self.after.replace('trtllm_request_success_total{', 'nv_trt_llm_request_success_total{'),
                     '\n'.join(l for l in self.after.splitlines() if '_bucket{' not in l or 'le="+Inf"' not in l),
                     self.after.replace('le="0.3"', 'le="0.31"'),
                     self.after.replace(' 2.0', ' 0.0'),
                     self.after.replace('finished_reason="stop"', 'finished_reason="length"')]:
            with self.assertRaises(ValueError):
                runner.compare(self.page, page, 1)
        with self.assertRaises(ValueError):
            runner.compare(self.page, self.after, 2)

    def test_memory_fit_uses_conservative_profiled_headroom(self):
        log = ('available KV cache memory when calculating max tokens: 30.00 GiB, '
               'fraction is set 0.9, kv size is 98304. device total memory 80.00 GiB')
        self.assertGreater(runner.headroom(log)['conservative_headroom_fraction'], .25)
        for bad in ['', log+'\n'+log, log.replace('30.00', '19.00'), log.replace('30.00', '20.00')]:
            with self.assertRaises(ValueError):
                runner.headroom(bad)


if __name__ == '__main__':
    unittest.main()
