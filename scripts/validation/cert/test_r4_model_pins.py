# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Fail-closed prospective catalog r4 lane and case model routing."""
import unittest

import r4_model_pins as pins


class ModelPinsTests(unittest.TestCase):
    def test_exact_pins_and_case_specific_g5(self):
        general = {'model': pins.GENERAL_MODEL, 'revision': pins.GENERAL_REVISION, 'image': pins.IMAGE}
        small = {'model': pins.SMALL_MODEL, 'revision': pins.SMALL_REVISION, 'image': pins.IMAGE}
        for lane in pins.GENERAL_LANES:
            pins.check(general, lane)
            with self.subTest(lane=lane), self.assertRaises(ValueError):
                pins.check(small, lane)
        pins.check(small, 'G7')
        pins.check(small, 'G5', 'BC-GPU-015')
        for case in ('BC-GPU-013', 'BC-GPU-014'):
            pins.check(general, 'G5', case)
            with self.assertRaises(ValueError):
                pins.check(small, 'G5', case)
        with self.assertRaises(ValueError):
            pins.check(general, 'G5', 'BC-GPU-015')
        for case in (None, 'BC-GPU-999'):
            with self.assertRaises(ValueError):
                pins.check(general, 'G5', case)
        for key in ('model', 'revision', 'image'):
            bad = dict(general, **{key: 'unexpected'})
            with self.subTest(key=key), self.assertRaises(ValueError):
                pins.check(bad, 'D', 'BC-COMP-002')
        # Existing immutable performance gate is not modified by r4 routing.
        pins.check({'model': 'historic'}, 'P1', 'BC-PERF-001')


if __name__ == '__main__':
    unittest.main()
