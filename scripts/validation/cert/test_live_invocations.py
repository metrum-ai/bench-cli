# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""CPU-only checks of the real live runners' command assembly."""
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

import l1_runner
import live_modality_runner


class LiveInvocationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.prompt = self.root / 'input.jsonl'
        self.prompt.write_text('a synthetic image-generation prompt\n')
        self.args = SimpleNamespace(candidate=self.root / 'bin/metrum-ai-bench-cli-llm',
                                    port=19000, model='synthetic/model', sut=self.root / 'sut.json',
                                    prompt=self.prompt, modality='vlm')
        self.out = self.root / 'out'

    def test_l1_command_uses_frozen_arguments(self):
        command, additions = l1_runner.smoke_invocation(self.args, self.out)
        self.assertEqual(command[:4], ['cargo', '--locked', 'xtask', 'local-smoke'])
        self.assertEqual(command[4], 'llm')
        self.assertEqual(command[command.index('--sut') + 1], str(self.args.sut))
        self.assertEqual(command[command.index('--out') + 1], str(self.out))
        self.assertEqual(command[command.index('--num-requests') + 1], '16')
        self.assertEqual(additions['PROMPT_LOCAL_JSONL'], str(self.prompt))
        self.assertEqual(additions['BENCH_BIN_DIR'], str(self.args.candidate.parent))

    def test_modality_commands_and_image_prompt(self):
        for modality, count in (('vlm', '8'), ('asr', '6'), ('imagegen', '4')):
            self.args.modality = modality
            with self.subTest(modality=modality):
                command, headline, additions = live_modality_runner.smoke_invocations(self.args, self.out)
                self.assertEqual(command[4], modality)
                self.assertEqual(command[command.index('--sut') + 1], str(self.args.sut))
                self.assertEqual(command[command.index('--num-requests') + 1], count)
                self.assertEqual(headline[:4], ['cargo', '--locked', 'xtask', 'assert-headline'])
                self.assertEqual(additions['CARGO_NET_OFFLINE'], 'true')
                self.assertEqual('--artifact-dir' in headline, modality == 'imagegen')
                self.assertEqual('IMAGEGEN_PROMPT' in additions, modality == 'imagegen')
        self.args.modality = 'imagegen'
        self.prompt.write_text('')
        with self.assertRaises(ValueError):
            live_modality_runner.smoke_invocations(self.args, self.out)


if __name__ == '__main__':
    unittest.main()
