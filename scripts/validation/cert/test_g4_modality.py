# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""CPU-only modality identity and math checks, never GPU evidence."""
import hashlib
import json
import tempfile
import unittest
from pathlib import Path

import g4_modality_checks as checks
import g4_modality_runner as runner
import independent_distributions as distributions


class ModalityTests(unittest.TestCase):
    def test_pinned_local_image_only(self):
        with tempfile.TemporaryDirectory() as directory:
            image = Path(directory) / 'image.png'; image.write_bytes(b'fixture')
            prompts = Path(directory) / 'prompts.jsonl'
            prompts.write_text(json.dumps({'prompt': 'describe', 'image_urls': [str(image)]}))
            runner.image_prompts(prompts, image)
            prompts.write_text(json.dumps({'prompt': 'describe', 'image_urls': ['https://example.invalid/image.png']}))
            with self.assertRaises(ValueError): runner.image_prompts(prompts, image)

    def test_trace_identity_rejects_missing_or_wrong_image(self):
        digest = hashlib.sha256(b'fixture').hexdigest()
        events = [{'request': 1, 'kind': 'image_identity', 'image_sha256': [digest], 'model': 'm', 'thinking': True},
                  {'request': 1, 'kind': 'send'}]
        trace = '\n'.join(map(json.dumps, events))
        self.assertEqual(1, checks.verify_images(trace, digest, 'm'))
        with self.assertRaises(ValueError): checks.verify_images(trace, '0' * 64, 'm')
        events[0]['thinking'] = False
        with self.assertRaises(ValueError): checks.verify_images('\n'.join(map(json.dumps, events)), digest, 'm')

    def test_independent_rate_distribution(self):
        summary = {'rate': {'n': 2, 'min': 1, 'max': 3, 'avg': 2, 'std': 2 ** .5, 'mad': 1,
                            'p50': 2, 'p90': 2.8, 'p95': 2.9, 'p99': 2.98, 'percentile_method': 'hyndman_fan_type7'}}
        self.assertEqual(2, distributions.verify(summary, 'rate', [1, 3]))
        summary['rate']['p95'] = 9
        with self.assertRaises(ValueError): distributions.verify(summary, 'rate', [1, 3])


if __name__ == '__main__':
    unittest.main()
