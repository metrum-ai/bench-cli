# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic two-model barrier tests, never a GPU certification result."""
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import manifest_tool
import perf_comp001_barrier as barrier
import test_perf_evidence

PERF = Path(__file__).resolve().parent.parent / 'perf/test_compare.py'
spec = importlib.util.spec_from_file_location('perf_fixtures_barrier', PERF)
fixtures = importlib.util.module_from_spec(spec); spec.loader.exec_module(fixtures)


def bundle(root, lane, model, revision, manifest_hash, candidate_hash, baseline_hash):
    root.mkdir(); (root / 'results').mkdir(); evidence = root / 'evidence'; evidence.mkdir()
    perf = evidence / 'perf'; test_perf_evidence.grid(perf)
    gpu = 'NVIDIA RTX PRO 6000 Blackwell Server Edition'
    (evidence / 'gpu.csv').write_text(f'{gpu}, fixture-driver\n')
    (evidence / 'gpu-identity.csv').write_text(f'{gpu}, GPU-fixture, 98304\n')
    (evidence / 'cleanup-verified.json').write_text('{"cleanup":"verified"}\n')
    (perf / 'models.json').write_text(json.dumps({'data': [{'id': model}]}))
    (perf / 'vllm.log').write_text('FlashInferCutlassNvFp4LinearKernel')
    for label in ('bench-rel', 'bench-src'):
        binary_hash = baseline_hash if label == 'bench-rel' else candidate_hash
        for workload in ('W1', 'W2'):
            for concurrency in (1, 8, 32):
                base = perf / label / '1' / workload / f'c{concurrency}'
                for folder in (base, base / 'warmup'):
                    command_path = folder / 'command.json'; command = json.loads(command_path.read_text())
                    command['binary_sha256'] = command['binary_sha256_end'] = binary_hash
                    command['argv'][command['argv'].index('--model') + 1] = model
                    command['argv'] += ['--prompts', str(root.parent / f'{workload}.jsonl')]
                    command_path.write_text(json.dumps(command))
                saved = (base / 'point.json').read_text()
                fixtures.write_point(perf / label, workload, concurrency, 1, tok_s=100, tpot_ms=40)
                (base / 'point.json').write_text(saved)
                data = base / 'requests.jsonl'; rows = [json.loads(line) for line in data.read_text().splitlines()]
                rows[-1].update({'attempted': 8, 'successes': 8, 'errors': 0, 'usage_missing_count': 0,
                                 'completion_tokens_total': 24, 'prompt_tokens_total': 800,
                                 'completion_tokens_per_second': 100, 'ttft_coverage': {'measured': 8},
                                 'ttft_first_any_s': {'n': 8, 'p50': .05}})
                data.write_text(''.join(json.dumps(row) + '\n' for row in rows))
                sequence = [{'seq': i, 'cohort': i // concurrency, 'prompt_sha256_16': 'a' * 16, 'prompt_bytes': 12} for i in range(8)]
                (base / 'prompt-sequence.jsonl').write_text(''.join(json.dumps(row) + '\n' for row in sequence))
    code, output, report = fixtures.run_compare(perf / 'bench-rel', perf / 'bench-src')
    if code: raise AssertionError(output)
    report['model'] = 'q38-27b' if lane == 'P1' else 'q38-27b-nvfp4'
    (perf / 'comparison.json').write_text(json.dumps(report)); (perf / 'comparison.md').write_text('fixture report')
    case = 'BC-PERF-001' if lane == 'P1' else 'BC-PERF-002'
    record = {'ID': case, 'result': 'PASS', 'candidate_sha': 'a' * 40, 'gpu_class': gpu, 'driver': 'fixture-driver',
              'evidence_paths': ['evidence/cleanup-verified.json'],
              'versions': {'manifest_sha256': manifest_hash, 'candidate_binary_sha256': candidate_hash,
                           'baseline_binary_sha256': baseline_hash, 'baseline_release': 'v1.7.2',
                           'model': model, 'model_revision': revision, 'server_image': 'image'}}
    (root / 'results' / f'{case}.json').write_text(json.dumps(record))


class BarrierTests(unittest.TestCase):
    def test_both_models_required_and_copied_evidence_rechecked(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); b = 'b' * 64; c = 'c' * 64
            config = lambda model, revision: {'model': model, 'revision': revision, 'image': 'image',
                         'prompt_sets': {w: {'sha256': w, 'path': str(root / f'{w}.jsonl')} for w in ('W1', 'W2')}}
            manifest = {'candidate_sha': 'a' * 40, 'candidate_binary_sha256': c, 'baseline_binary_sha256': b,
                        'lanes': {'P1': config('model-p1', 'rev1'), 'P2': config('model-p2', 'rev2')}}
            file = root / 'manifest.json'; file.write_text(json.dumps(manifest)); digest = manifest_tool.sha256_file(file)
            bundle(root / 'a', 'P1', 'model-p1', 'rev1', digest, c, b)
            bundle(root / 'b', 'P2', 'model-p2', 'rev2', digest, c, b)
            def invoke(output, p2):
                argv = ['barrier', '--p1-out', str(root / 'a'), '--p2-out', str(p2), '--manifest', str(file),
                        '--out', str(output), '--owner', 'fixture', '--evidence-link', 'fixture']
                with patch('sys.argv', argv), patch.dict(os.environ, {}, clear=False):
                    return barrier.main()
            self.assertEqual(invoke(root / 'out-fail', root / 'missing'), 1)
            self.assertEqual(json.loads((root / 'out-fail/results/BC-COMP-001.json').read_text())['result'], 'FAIL')
            self.assertEqual(invoke(root / 'out-pass', root / 'b'), 0)
            result = json.loads((root / 'out-pass/results/BC-COMP-001.json').read_text())
            self.assertEqual(result['result'], 'PASS'); self.assertEqual(result['observed_values']['comparable_points'], 12)
            (root / 'b/results/lane-error.json').write_text('{}')
            self.assertEqual(invoke(root / 'out-cleanup-fail', root / 'b'), 1)
            (root / 'b/results/lane-error.json').unlink()
            seq = root / 'b/evidence/perf/bench-src/1/W1/c8/prompt-sequence.jsonl'
            seq.write_text(seq.read_text().replace('a' * 16, 'f' * 16, 1))
            self.assertEqual(invoke(root / 'out-prompt-fail', root / 'b'), 1)


if __name__ == '__main__': unittest.main()
