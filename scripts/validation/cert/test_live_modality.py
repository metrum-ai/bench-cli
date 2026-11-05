# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline live-modality shape tests; never GPU evidence."""
import json
import unittest

import live_modality_checks as checks


def log(metrics, error=None):
    rows = [{'schema_version': 'metrum.request.v3', 'phase': 'measure', 'error': error, 'modality_metrics': metrics},
            {'schema_version': 'metrum.summary.v3', 'attempted': 1, 'successes': 1,
             'sut': {'model': 'fixture'}, 'config': {'common': {'require_sut': True}}}]
    return '\n'.join(json.dumps(row) for row in rows)


class LiveChecksTests(unittest.TestCase):
    def test_three_supported_modalities(self):
        for kind, metrics, count in [('vlm', {'image_count': 1}, 0), ('asr', {'wer': .1, 'cer': .2}, 0),
                                     ('imagegen', {'images_returned': 1}, 1)]:
            self.assertEqual(checks.evaluate(log(metrics), kind, 0, 0, count)['attempted'], 1)

    def test_failure_missing_metric_and_artifact(self):
        with self.assertRaises(ValueError): checks.evaluate(log({'image_count': 0}), 'vlm', 0, 0, 0)
        with self.assertRaises(ValueError): checks.evaluate(log({'wer': 0}), 'asr', 0, 0, 0)
        with self.assertRaises(ValueError): checks.evaluate(log({'images_returned': 2}), 'imagegen', 0, 0, 1)
        with self.assertRaises(ValueError): checks.evaluate(log({'image_count': 1}), 'vlm', 0, 1, 0)
        with self.assertRaises(ValueError): checks.evaluate(log({'image_count': 1}, {'kind': 'transport'}), 'vlm', 0, 0, 0)


if __name__ == '__main__': unittest.main()
