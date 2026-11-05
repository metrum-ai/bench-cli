# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Optional CPU-only integration with actual release and candidate CLIs.

Set CERT_TEST_BASELINE_BIN and CERT_TEST_LLM_BIN to executable binaries. This
synthetic loopback server is not GPU evidence or a performance measurement.
"""
import json
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest

import compare


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
                               '16', '--temperature', '0', '--seed', '42', '--extra-body-json', '{"seed":42}']
                    result = subprocess.run(command, capture_output=True, text=True, timeout=60)
                    self.assertEqual(result.returncode, 0, result.stderr[-4000:])
                    (out / 'DONE').touch()
                    points[label] = compare.extract_point(out)
                    self.assertEqual(points[label]['sample_count'], 64)
                    self.assertGreater(points[label]['system_output_tok_s'], 0)
                    self.assertGreater(points[label]['tpot_p50_ms'], 0)
                headline = points['candidate']['headline']
                self.assertEqual(headline['ttft_first_any_s']['n'], 64)
                self.assertGreater(headline['completion_tokens_per_second'], 0)
            finally:
                server.shutdown(); server.server_close(); thread.join()


if __name__ == '__main__': unittest.main()
