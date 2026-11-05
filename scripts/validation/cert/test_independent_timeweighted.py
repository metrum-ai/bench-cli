# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""CPU-only interval oracle examples, not GPU evidence."""
import unittest
import independent_timeweighted as check


class TimeWeightedTests(unittest.TestCase):
    def test_two_overlapping_intervals(self):
        spans = [{'start': 10, 'latency': 2, 'first': .5, 'prompt': 100, 'output': 30},
                 {'start': 11, 'latency': 2, 'first': 1.5, 'prompt': 50, 'output': 10}]
        values = check.calculate(spans, 10, 4)
        self.assertAlmostEqual(1, values['effective_concurrency']['avg'])
        self.assertAlmostEqual(2, values['effective_concurrency']['max'])
        self.assertAlmostEqual(37.5, values['effective_prefill_throughput']['avg'])
        self.assertAlmostEqual(200, values['effective_prefill_throughput']['max'])
        self.assertAlmostEqual(10, values['effective_decode_throughput']['avg'])
        check.verify(values, spans, 10, 4)
        values['effective_decode_throughput']['avg'] = 0
        with self.assertRaises(ValueError): check.verify(values, spans, 10, 4)

    def test_zero_contributors_are_null(self):
        spans = [{'start': 1, 'latency': 1, 'first': None, 'prompt': None, 'output': None}]
        values = check.calculate(spans, 1, 1)
        self.assertEqual(0, values['effective_decode_throughput']['n'])
        self.assertIsNone(values['effective_decode_throughput']['avg'])


if __name__ == '__main__':
    unittest.main()
