# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline preparation checks; these are not GPU launch evidence."""
import json
import tempfile
import unittest
from pathlib import Path

import g4_modality_runner
import manifest_tool
import prepare_vlm_input


class PrepareVlmInputTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.image = self.root / 'image.png'
        self.image.write_bytes(b'fixture image')
        self.source = self.root / 'prompts.jsonl'
        self.source.write_text(json.dumps({'prompt': 'describe', 'image_url': 'image.png'}) + '\n')
        self.out = self.root / 'absolute.jsonl'

    def prepare(self, source_hash=None, image_hash=None):
        return prepare_vlm_input.prepare(self.source, source_hash or manifest_tool.sha256_file(self.source),
                                         self.image, image_hash or manifest_tool.sha256_file(self.image),
                                         self.root, self.out)

    def test_absolute_input_is_pinned_and_accepted_by_real_runner_validation(self):
        pin = self.prepare()
        self.assertEqual(pin['sha256'], manifest_tool.sha256_file(self.out))
        self.assertEqual(json.loads(self.out.read_text()), {'prompt': 'describe', 'image_url': str(self.image)})
        g4_modality_runner.image_prompts(self.out, self.image.resolve())
        with self.assertRaises(FileExistsError):
            self.prepare()

    def test_source_and_image_pins_fail_closed(self):
        for kwargs in ({'source_hash': '0' * 64}, {'image_hash': '0' * 64}):
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                self.prepare(**kwargs)
        self.assertFalse(self.out.exists())

    def test_rejects_remote_unpinned_multiple_and_ambiguous_images(self):
        other = self.root / 'other.png'
        other.write_bytes(b'other')
        for value in ({'image_url': 'https://example.invalid/image.png'},
                      {'image_url': str(other)}, {'image_urls': ['image.png', 'image.png']},
                      {'image_url': 'image.png', 'image_urls': ['image.png']}, {}):
            with self.subTest(value=value):
                self.source.write_text(json.dumps({'prompt': 'x', **value}) + '\n')
                with self.assertRaises(ValueError):
                    self.prepare()
                self.assertFalse(self.out.exists())

    def test_tracked_fixture_is_rewritten_and_accepted(self):
        repo = Path(__file__).resolve().parents[3]
        source = repo / 'test-data/vlm/prompts.jsonl'
        image = repo / 'test-data/vlm/shapes-512.png'
        prepare_vlm_input.prepare(source, manifest_tool.sha256_file(source), image,
                                  manifest_tool.sha256_file(image), repo, self.out)
        g4_modality_runner.image_prompts(self.out, image.resolve())


if __name__ == '__main__':
    unittest.main()
