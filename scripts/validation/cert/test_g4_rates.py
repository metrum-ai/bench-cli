# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic independent rate checks; no GPU result is implied."""
import json
import unittest

import g4_rates_checks
import test_g3_checks


class RatesTests(unittest.TestCase):
    def fixture(self):
        data, trace = test_g3_checks.G3ChecksTests().fixture()
        rows = [json.loads(line) for line in data.splitlines()]
        for i, row in enumerate(rows[:-1]):
            row.update({'usage_missing': i == 3, 'prompt_tokens': 2 if i != 3 else 0,
                        'completion_tokens': 3 if i != 3 else 0,
                        'visible_completion_tokens': 1 if i != 3 else None})
        rows[-1].update({'prompt_tokens_total': None, 'completion_tokens_total': None,
                         'input_tokens_per_second': None, 'completion_tokens_per_second': None,
                         'total_tokens_per_second': None, 'non_reasoning_completion_tokens_per_second': None,
                         'legacy_success_completion_tokens_per_second': 6 / 3.5})
        return '\n'.join(map(json.dumps, rows)), trace

    def test_independent_rates_and_missing_usage(self):
        data, trace = self.fixture()
        observed = g4_rates_checks.verify(data, trace, 2.5)
        self.assertEqual(2, observed['failure_answer_overlaps'])
        self.assertFalse(observed['usage_complete'])
        with self.assertRaises(ValueError):
            g4_rates_checks.verify(data.replace('"input_tokens_per_second": null', '"input_tokens_per_second": 2'), trace, 2.5)
        with self.assertRaises(ValueError):
            g4_rates_checks.verify(data.replace('"legacy_success_completion_tokens_per_second": 1.7142857142857142', '"legacy_success_completion_tokens_per_second": 9'), trace, 2.5)


if __name__ == '__main__':
    unittest.main()
