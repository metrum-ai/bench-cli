#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline exact arithmetic controls; rates and budgets come from reviewed inputs."""
import unittest
from fractions import Fraction

import stage0_limits as limits


class LimitTests(unittest.TestCase):
    def test_box_cap_is_exactly_five_fourths(self):
        self.assertEqual(limits.duration_cap(3600), Fraction(4500))
        self.assertEqual(limits.duration_cap(1), Fraction(5, 4))
        for value in (0, -1, True, 1.5, '3600'):
            with self.subTest(value=value), self.assertRaises(ValueError):
                limits.duration_cap(value)

    def test_backstop_uses_rate_cap_and_contingency_exactly(self):
        self.assertEqual(limits.backstop_exposure('4', 3600), Fraction(11, 2))
        third = limits.backstop_exposure('0.01', 1)
        self.assertGreater(third, 0)
        self.assertEqual(limits.ceil_cents(third), 1)
        self.assertEqual(limits.ceil_cents(Fraction(1, 100)), 1)

    def test_stop_is_strict_and_unknown_is_not_zero(self):
        self.assertEqual(limits.require_below_stop(incurred='20', committed='30', proposed='49.99', stop='100'), Fraction(9999, 100))
        for values in [dict(incurred='20', committed='30', proposed='50', stop='100'),
                       dict(incurred=None, committed='30', proposed='10', stop='100'),
                       dict(incurred='0', committed='0', proposed='0', stop='0')]:
            with self.subTest(values=values), self.assertRaises(ValueError):
                limits.require_below_stop(**values)

    def test_amounts_reject_float_nan_and_exponents(self):
        for value in (0.1, True, 'nan', 'Infinity', '-1', '1e9', ' 1', '01', ''):
            with self.subTest(value=value), self.assertRaises(ValueError):
                limits.amount(value)


if __name__ == '__main__':
    unittest.main()
