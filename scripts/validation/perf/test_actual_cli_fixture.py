# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Optional CPU-only integration with actual release and candidate CLIs.

Set CERT_TEST_BASELINE_BIN and CERT_TEST_LLM_BIN to executable binaries. This
synthetic loopback server is not GPU evidence or a performance measurement.
"""
import importlib.util
import json
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import threading
import unittest

import compare

spec = importlib.util.spec_from_file_location('perf_legacy_oracle', Path(__file__).resolve().parent.parent / 'cert/perf_legacy_oracle.py')
oracle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(oracle)


class Fixture(BaseHTTPRequestHandler):
    def log_message(self, *_args): pass

    def do_POST(self):
        self.rfile.read(int(self.headers['Content-Length']))
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.end_headers()
        for data in (
            {'choices': [{'index': 0, 'delta': {'role': 'assistant'}}]},
            {'choices': [{'index': 0, 'delta': {'content': 'Hello'}}]},
            {'choices': [{'index': 0, 'delta': {'content': ' world'}}]},
            {'choices': [{'index': 0, 'delta': {}, 'finish_reason': 'stop'}],
             'usage': {'prompt_tokens': 5, 'completion_tokens': 2, 'total_tokens': 7}},
        ):
            self.wfile.write(('data: ' + json.dumps(data) + '\n\n').encode())
            self.wfile.flush()
        self.wfile.write(b'data: [DONE]\n\n')
        self.wfile.flush()


@unittest.skipUnless(os.getenv('CERT_TEST_BASELINE_BIN') and os.getenv('CERT_TEST_LLM_BIN'),
                     'actual baseline and candidate fixture binaries not supplied')
class ActualCliFixture(unittest.TestCase):
    def test_both_actual_clis_support_legacy_comparison(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); prompt = root / 'prompts.jsonl'
            prompt.write_text('{"prompt":"Say hello."}\n', encoding='utf-8')
            server = ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
            thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
            try:
                points = {}
                for label, variable in (('baseline', 'CERT_TEST_BASELINE_BIN'), ('candidate', 'CERT_TEST_LLM_BIN')):
                    out = root / label; out.mkdir()
                    (out / 'point.json').write_text(json.dumps({'workload': 'W1', 'concurrency': 1}))
                    command = [os.environ[variable], '--scenario', 'W1', '--url',
                               f'http://127.0.0.1:{server.server_port}/v1/chat/completions', '--api-key', 'dummy',
                               '--num-requests', '64', '--concurrency', '1', '--prompts', str(prompt), '--mode', 'chat',
                               '--streaming', '--model', 'dummy', '--data-log', str(out / 'requests.jsonl'), '--max-tokens',
                               '16', '--temperature', '0', '--seed', '42', '--extra-body-json', '{"seed":42}',
                               '--prompt-sequence-log', str(out / 'prompt-sequence.jsonl')]
                    result = subprocess.run(command, capture_output=True, text=True, timeout=60)
                    self.assertEqual(result.returncode, 0, result.stderr[-4000:])
                    (out / 'DONE').touch()
                    points[label] = compare.extract_point(out)
                    self.assertEqual(points[label]['sample_count'], 64)
                    self.assertGreater(points[label]['system_output_tok_s'], 0)
                    self.assertGreater(points[label]['tpot_p50_ms'], 0)
                    independent = oracle.point(out)
                    self.assertEqual(independent['successes'], points[label]['sample_count'])
                    self.assertAlmostEqual(independent['legacy_output_tok_s'], points[label]['system_output_tok_s'])
                    self.assertAlmostEqual(independent['legacy_tpot_p50_ms'], points[label]['tpot_p50_ms'])
                left = (root / 'baseline/prompt-sequence.jsonl').read_text().splitlines()
                right = (root / 'candidate/prompt-sequence.jsonl').read_text().splitlines()
                self.assertEqual(len(left), 64)
                self.assertEqual(left, right)
                headline = points['candidate']['headline']
                self.assertEqual(headline['ttft_first_any_s']['n'], 64)
                self.assertGreater(headline['completion_tokens_per_second'], 0)
                # Duplicate captured synthetic CLI points to exercise the grid
                # oracle, never as performance or GPU evidence.
                for label, source in (('bench-rel', root / 'baseline'), ('bench-src', root / 'candidate')):
                    for workload in ('W1', 'W2'):
                        for concurrency in (1, 8, 32):
                            dest = root / label / '1' / workload / f'c{concurrency}'
                            shutil.copytree(source, dest)
                            (dest / 'point.json').write_text(json.dumps({'workload': workload, 'concurrency': concurrency}))
                report_path = root / 'comparison.json'
                subprocess.run([sys.executable, str(Path(__file__).with_name('compare.py')),
                                '--baseline-root', str(root / 'bench-rel'), '--candidate-root', str(root / 'bench-src'),
                                '--model', 'q38-27b', '--pass', '1', '--tolerance-pct', '5', '--out-json', str(report_path)],
                               capture_output=True, text=True, check=False)
                report = json.loads(report_path.read_text())
                # Synthetic timing may exceed the gate; the oracle still must
                # reconstruct the reported raw native headlines and legacy grid.
                independent = oracle.verify_grid(root, report, 'q38-27b')
                self.assertEqual(len(independent['points']), 6)
            finally:
                server.shutdown(); server.server_close(); thread.join()


if __name__ == '__main__': unittest.main()
