#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Exercise the pinned LLM binary's real --require-sut parser on loopback only.

This narrow probe is not a selected-case rehearsal or a GPU qualification. It
uses temporary output and a disposable CPU-only server, never case records.
"""
import argparse
import json
import os
import re
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import manifest_tool


def probe(binary, binary_sha, sut_path, sut_sha):
    binary, sut_path = Path(binary).resolve(strict=True), Path(sut_path).resolve(strict=True)
    for path, expected in ((binary, binary_sha), (sut_path, sut_sha)):
        if not re.fullmatch(r'[a-f0-9]{64}', expected) or manifest_tool.sha256_file(path) != expected:
            raise ValueError('probe input hash mismatch')
    if not os.access(binary, os.X_OK):
        raise ValueError('candidate is not executable')
    sut = json.loads(sut_path.read_text(encoding='utf-8'), object_pairs_hook=manifest_tool.unique_object)
    model = sut['model']['id']
    response = json.dumps({'id': 'offline', 'object': 'chat.completion', 'created': 1, 'model': model,
                           'choices': [{'index': 0, 'message': {'role': 'assistant', 'content': 'ok'},
                                        'finish_reason': 'stop'}],
                           'usage': {'prompt_tokens': 1, 'completion_tokens': 1, 'total_tokens': 2}}).encode()
    seen = []

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_POST(self):
            self.rfile.read(int(self.headers.get('Content-Length', '0')))
            seen.append(self.path)
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(response)))
            self.end_headers()
            self.wfile.write(response)

    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    server.daemon_threads = True
    worker = threading.Thread(target=server.serve_forever, kwargs={'poll_interval': .05}, daemon=True)
    worker.start()
    try:
        with tempfile.TemporaryDirectory(prefix='cert-product-sut-probe-') as temp:
            root = Path(temp)
            prompt, data = root / 'prompts.jsonl', root / 'data.jsonl'
            prompt.write_text('{"prompt":"offline parser check"}\n', encoding='utf-8')
            command = [str(binary), '--scenario', 'offline-sut-probe', '--url',
                       f'http://127.0.0.1:{server.server_port}/v1/chat/completions', '--api-key', 'offline-dummy',
                       '--num-requests', '1', '--concurrency', '1', '--prompts', str(prompt), '--mode', 'chat',
                       '--model', model, '--data-log', str(data), '--max-tokens', '1', '--warmup-requests', '0',
                       '--request-timeout', '10', '--sut', str(sut_path), '--require-sut']
            env = {'PATH': '', 'HOME': str(root), 'NO_PROXY': '*', 'no_proxy': '*',
                   'TOKENIZERS_PARALLELISM': 'false'}
            result = subprocess.run(command, capture_output=True, text=True, timeout=30, cwd=root, env=env)
            if result.returncode != 0 or not data.is_file():
                raise ValueError(f'product SUT probe failed (exit {result.returncode}): {result.stderr[-1000:]}')
            rows = [json.loads(line) for line in data.read_text(encoding='utf-8').splitlines() if line.strip()]
            requests = [row for row in rows if '.request.' in row.get('schema_version', '')]
            summaries = [row for row in rows if '.summary.' in row.get('schema_version', '')]
            if len(requests) != 1 or requests[0].get('error') is not None or len(summaries) != 1 or len(seen) != 1:
                raise ValueError('product SUT probe did not complete exactly one loopback request and summary')
            summary = summaries[0]
            recorded = summary.get('sut') or {}
            if (summary.get('config', {}).get('common', {}).get('require_sut') is not True
                    or recorded.get('gpu', {}).get('model') != sut['gpu']['model']
                    or recorded.get('gpu', {}).get('count') != sut['gpu']['count']
                    or recorded.get('model', {}).get('id') != sut['model']['id']
                    or recorded.get('model', {}).get('revision') != sut['model']['revision']):
                raise ValueError('summary did not retain required SUT identity')
    finally:
        server.shutdown()
        server.server_close()
        worker.join(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('binary', 'binary-sha256', 'sut', 'sut-sha256'):
        parser.add_argument('--' + name, required=True)
    args = parser.parse_args()
    try:
        probe(args.binary, args.binary_sha256, args.sut, args.sut_sha256)
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as exc:
        parser.exit(1, f'LLM SUT probe failed: {exc}\n')
    print('LLM product --require-sut loopback probe passed; not a full case rehearsal')


if __name__ == '__main__':
    main()
