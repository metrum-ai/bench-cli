#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline baseline-evidence controls; these do not run a performance server."""
import hashlib
import json
import tempfile
import unittest
from pathlib import Path

import stage0_preview_baseline as preview


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class PreviewBaselineTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name) / 'run'; self.root.mkdir()
        self.binary = Path(self.tmp.name) / 'baseline'; self.binary.write_text('baseline')
        self.prompt = Path(self.tmp.name) / 'prompt'; self.prompt.write_text('prompt')
        self.hash = digest(self.binary); self.phash = digest(self.prompt)
        for suffix, duration in (('warmup', 30), ('', 180)):
            folder = self.root / 'bench-rel/1/W1/c1' / suffix; folder.mkdir(parents=True, exist_ok=True)
            (folder / 'DONE').touch()
            (folder / 'point.json').write_text(json.dumps({'tool': 'bench-rel', 'workload': 'W1',
                'concurrency': 1, 'pass': '1', 'start': 1767225600, 'stop': 1767225600 + duration}))
            args = {'--scenario': 'W1', '--concurrency': '1', '--model': 'model',
                    '--url': 'http://127.0.0.1:8000/v1/chat/completions', '--api-key': 'dummy',
                    '--max-tokens': '128', '--temperature': '0', '--seed': '42', '--num-requests': '1000000',
                    '--stop-after-seconds': str(duration), '--mode': 'chat', '--prompts': str(self.prompt),
                    '--data-log': str(folder / 'requests.jsonl'), '--extra-body-json': '{"seed":42}'}
            argv = [str(self.binary)] + [item for pair in args.items() for item in pair] + ['--streaming']
            (folder / 'command.json').write_text(json.dumps({'argv': argv, 'binary_sha256': self.hash,
                'binary_sha256_end': self.hash, 'prompt_sha256': self.phash, 'prompt_sha256_end': self.phash}))
        self.measure = self.root / 'bench-rel/1/W1/c1'
        self.row = {'schema_version': 'metrum-ai-bench-cli.request.v5', 'phase': 'measure', 'outcome': 'answer',
                    'answer_completed': True, 'started_at': '2026-01-01T00:00:00Z', 'completed_at': '2026-01-01T00:00:01Z'}
        (self.measure / 'requests.jsonl').write_text(json.dumps(self.row) + '\n')

    def verify(self):
        return preview.verify_p1p2(self.root, baseline=str(self.binary), baseline_sha256=self.hash,
                                   prompt=str(self.prompt), prompt_sha256=self.phash, model='model')

    def test_completed_baseline_only(self):
        self.assertEqual(self.verify()['baseline_cell'], 'bench-rel/1/W1/c1')

    def test_missing_done_or_short_duration_fails(self):
        (self.measure / 'warmup/DONE').unlink()
        with self.assertRaises(ValueError): self.verify()
        (self.measure / 'warmup/DONE').touch()
        point = self.measure / 'point.json'; value = json.loads(point.read_text()); value['stop'] = 1767225779
        point.write_text(json.dumps(value))
        with self.assertRaisesRegex(ValueError, 'duration'): self.verify()

    def test_bad_command_or_input_fails(self):
        path = self.measure / 'command.json'; command = json.loads(path.read_text())
        command['argv'].extend(['--concurrency', '8']); path.write_text(json.dumps(command))
        with self.assertRaises(ValueError): self.verify()
        self.binary.write_text('different')
        with self.assertRaisesRegex(ValueError, 'pin differs'): self.verify()

    def test_incomplete_or_wrong_request_fails(self):
        for key, value in (('answer_completed', False), ('outcome', 'error'), ('phase', 'warmup'),
                           ('started_at', '2025-12-31T23:59:59Z')):
            with self.subTest(key=key):
                row = dict(self.row); row[key] = value
                (self.measure / 'requests.jsonl').write_text(json.dumps(row) + '\n')
                with self.assertRaisesRegex(ValueError, 'no completed'): self.verify()

    def test_symlinked_evidence_rejected(self):
        file = self.measure / 'requests.jsonl'; target = self.root / 'other.jsonl'
        target.write_text(file.read_text()); file.unlink(); file.symlink_to(target)
        with self.assertRaisesRegex(ValueError, 'symlinked'): self.verify()


if __name__ == '__main__': unittest.main()
