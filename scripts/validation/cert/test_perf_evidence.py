# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""CPU fixtures for perf evidence structure; no GPU result is produced."""
import json
import tempfile
import unittest
from pathlib import Path

import perf_evidence_checks as checks


def grid(root):
    for workload, cap in (('W1', 128), ('W2', 512)):
        for concurrency in (1, 8, 32):
            for label in ('bench-rel', 'bench-src'):
                base = root / label / '1' / workload / f'c{concurrency}'
                for phase, duration in (('warmup', 30), ('measure', 180)):
                    directory = base / 'warmup' if phase == 'warmup' else base
                    directory.mkdir(parents=True, exist_ok=True)
                    (directory / 'DONE').touch()
                    (directory / 'point.json').write_text(json.dumps({'tool': label, 'workload': workload,
                        'concurrency': concurrency, 'pass': '1', 'start': 100, 'stop': 100 + duration}))
                    argv = ['binary', '--scenario', workload, '--concurrency', str(concurrency), '--model', 'model',
                            '--url', 'http://127.0.0.1:8000/v1/chat/completions', '--api-key', 'dummy',
                            '--max-tokens', str(cap), '--temperature', '0', '--seed', '42', '--num-requests', '1000000',
                            '--stop-after-seconds', str(duration), '--mode', 'chat', '--streaming', '--extra-body-json', '{"seed":42}']
                    if phase == 'measure':
                        sequence = directory / 'prompt-sequence.jsonl'
                        entries = [{'seq': i, 'cohort': i // concurrency, 'prompt_sha256_16': 'a' * 16,
                                    'prompt_bytes': 12} for i in range(2)]
                        sequence.write_text(''.join(json.dumps(item) + '\n' for item in entries))
                        requests = [{'schema_version': 'bench.request.v1', 'phase': 'measure', 'seq': i} for i in range(2)]
                        requests.append({'schema_version': 'bench.summary.v1', 'attempted': 2, 'partial': False})
                        (directory / 'requests.jsonl').write_text(''.join(json.dumps(item) + '\n' for item in requests))
                        argv += ['--prompt-sequence-log', str(sequence)]
                    digest = 'b' if label == 'bench-rel' else 'c'
                    (directory / 'command.json').write_text(json.dumps({'argv': argv, 'binary_sha256': digest,
                           'binary_sha256_end': digest, 'prompt_sha256': workload, 'prompt_sha256_end': workload}))


class PerfEvidenceTests(unittest.TestCase):
    def test_full_grid_and_missing_or_short_phase(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); grid(root)
            self.assertEqual(checks.verify(root, 'model', 'c', 'b', {'W1': 'W1', 'W2': 'W2'})['verified_phases'], 24)
            self.assertEqual(checks.verify_sequences(root)['matched_points'], 6)
            point = root / 'bench-src/1/W2/c32/point.json'
            value = json.loads(point.read_text()); value['stop'] = 279.99; point.write_text(json.dumps(value))
            with self.assertRaises(ValueError): checks.verify(root, 'model', 'c', 'b', {'W1': 'W1', 'W2': 'W2'})

    def test_different_effective_sequence_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); grid(root)
            sequence = root / 'bench-src/1/W2/c8/prompt-sequence.jsonl'
            sequence.write_text(sequence.read_text().replace('a' * 16, 'b' * 16, 1))
            with self.assertRaises(ValueError): checks.verify_sequences(root)

    def test_candidate_native_headlines_are_required_but_informational(self):
        headline = {'window_seconds': 180, 'completion_tokens_per_second': 5, 'ttft_first_any_s': {'n': 3, 'p50': 0.2},
                    'attempted': 3, 'successes': 3, 'errors': 0, 'usage_missing_count': 0,
                    'completion_tokens_total': 900, 'prompt_tokens_total': 30, 'ttft_coverage': {'measured': 3}}
        points = [{'candidate': {'headline': headline}} for _ in range(6)]
        checks.verify_candidate_headlines(points)
        headline['ttft_first_any_s'] = None
        with self.assertRaises(ValueError): checks.verify_candidate_headlines(points)

    def test_wrong_prompt_or_binary_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); grid(root)
            with self.assertRaises(ValueError): checks.verify(root, 'model', 'wrong', 'b', {'W1': 'W1', 'W2': 'W2'})
            with self.assertRaises(ValueError): checks.verify(root, 'model', 'c', 'b', {'W1': 'wrong', 'W2': 'W2'})


if __name__ == '__main__': unittest.main()
