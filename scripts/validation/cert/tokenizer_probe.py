#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""CPU-only frozen-CLI tokenizer probe; never emits GPU case records."""
import json
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


def check_rows(data):
    rows = [json.loads(line) for line in data.splitlines() if line.strip()]
    rows = [row for row in rows if isinstance(row, dict) and '.request.' in str(row.get('schema_version', ''))]
    if len(rows) != 1:
        raise ValueError('offline tokenizer probe requires exactly one request record')
    row = rows[0]
    count = row.get('tokenized_completion_tokens')
    if row.get('error') is not None or row.get('answer_completed') is not True or type(count) is not int or count <= 0:
        raise ValueError('frozen candidate lacks usable tokenizer fallback; check tokenizer build feature and file')
    if row.get('usage_completion_tokens_present') is not False:
        raise ValueError('offline tokenizer probe must not obtain counts from server usage')


def probe(binary, tokenizer):
    binary, tokenizer = Path(binary).resolve(), Path(tokenizer).resolve()
    body = (b'data: {"choices":[{"index":0,"delta":{"content":"Hello tokenizer."}}]}\n\n'
            b'data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}\n\n'
            b'data: [DONE]\n\n')
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass
        def do_POST(self):
            self.rfile.read(int(self.headers.get('Content-Length', '0')))
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
    server = ThreadingHTTPServer(('localhost', 0), Handler)
    server.daemon_threads = True
    worker = threading.Thread(target=server.serve_forever, kwargs={'poll_interval': .05}, daemon=True)
    worker.start()
    try:
        with tempfile.TemporaryDirectory(prefix='cert-tokenizer-probe-') as temporary:
            root = Path(temporary)
            prompt, data = root / 'prompts.jsonl', root / 'requests.jsonl'
            prompt.write_text('{"prompt":"Offline tokenizer probe."}\n')
            command = [str(binary), '--scenario', 'offline-tokenizer-probe', '--url',
                       f'http://localhost:{server.server_port}/v1/chat/completions', '--api-key', 'offline-dummy',
                       '--num-requests', '1', '--concurrency', '1', '--prompts', str(prompt), '--mode', 'chat',
                       '--streaming', '--model', 'offline-fixture', '--data-log', str(data), '--max-tokens', '8',
                       '--warmup-requests', '0', '--request-timeout', '10', '--tokenizer', str(tokenizer)]
            # No inherited API credentials, exporter settings, proxy or Hub cache.
            environment = {'PATH': '', 'HOME': str(root),
                           'NO_PROXY': '*', 'no_proxy': '*', 'TOKENIZERS_PARALLELISM': 'false'}
            result = subprocess.run(command, capture_output=True, text=True, timeout=30, cwd=root, env=environment)
            if result.returncode != 0 or not data.is_file():
                raise ValueError(f'offline tokenizer probe failed (exit {result.returncode}): {result.stderr[-1000:]}')
            check_rows(data.read_text())
    finally:
        server.shutdown()
        server.server_close()
        worker.join(timeout=5)
