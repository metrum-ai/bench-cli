#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Partial GPU-013 strategic parser/argv loopback, never case evidence."""
import argparse
import json
import os
import socket
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from types import SimpleNamespace

import case_launcher
import g4_modality_runner
import g5_strategic_runner
import g6_counter_runner
import g7_runner
import l1_runner
import manifest_tool
import rehearse_live_modalities

CASE = 'BC-GPU-013'


def rehearsal(manifest_path):
    manifest_path = Path(manifest_path).resolve(strict=True)
    raw = manifest_path.read_bytes()
    manifest = json.loads(raw, object_pairs_hook=manifest_tool.unique_object)
    if list(manifest.get('cases', {})) != [CASE] or manifest.get('lane') != 'G5':
        raise ValueError('exactly the frozen GPU-013 G5 case is required')
    case_launcher.preflight(manifest)
    baseline = rehearse_live_modalities.parse_frozen_command(manifest, CASE, 'G5')
    candidate = Path(manifest['candidate']['path']).resolve(strict=True)
    _, config = manifest_tool.load_manifest(manifest['lane_manifest']['path'], 'G5', candidate, baseline, '0')
    strategic = g4_modality_runner.pinned_file(config, 'strategic_binary', 'metrum-ai-bench-cli-strategic', candidate)
    observed = []

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_GET(self):
            if self.path != '/health':
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header('Content-Length', '0')
            self.end_headers()

        def do_POST(self):
            if self.path != '/v1/chat/completions':
                self.send_error(404)
                return
            try:
                request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                observed.append(request)
                data = (b'data: {"choices":[{"index":0,"delta":{"content":"synthetic answer"}}]}\n\n'
                        b'data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],'
                        b'"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}\n\n'
                        b'data: [DONE]\n\n')
            except (ValueError, TypeError, KeyError):
                self.send_error(400)
                return
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Content-Length', str(len(data)))
            self.end_headers()
            self.wfile.write(data)

    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix='cert-g5-rehearsal-') as temporary:
            root = Path(temporary)
            evidence = root / 'evidence'
            evidence.mkdir()
            with socket.socket() as reserve:
                reserve.bind(('127.0.0.1', 0))
                proxy_port = reserve.getsockname()[1]
            args = SimpleNamespace(prompt=Path(config['prompt_sets']['default']['path']), model=config['model'],
                                   port=server.server_address[1], proxy_port=proxy_port)
            command, proxy_command, failure_command, trace, csv_path, failure_dir = g5_strategic_runner.invocations(args, strategic, evidence)
            env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(root), 'LANG': 'C.UTF-8',
                   'LC_ALL': 'C.UTF-8', 'NO_PROXY': 'localhost,127.0.0.1', 'no_proxy': 'localhost,127.0.0.1',
                   'HF_HUB_OFFLINE': '1', 'TRANSFORMERS_OFFLINE': '1'}
            with (evidence / 'proxy-error.txt').open('wb') as err:
                proxy = subprocess.Popen(proxy_command, stdout=subprocess.DEVNULL, stderr=err, env=env)
                try:
                    g6_counter_runner.wait_for_proxy(proxy_port, proxy)
                    code = l1_runner.call(command, evidence / 'stdout.json', evidence / 'stderr.txt', 900, env=env, cwd=root)
                    if code or len(observed) != 8 or not csv_path.is_file() or not trace.is_file():
                        raise ValueError(f'GPU-013 strategic command failed: exit={code}, requests={len(observed)}')
                    failure_dir.mkdir()
                    failure = l1_runner.call(failure_command, evidence / 'failure.json', evidence / 'failure.txt', 900, env=env, cwd=root)
                    if failure == 0 or len(observed) != 9:
                        raise ValueError('GPU-013 expected export failure did not exercise a product request')
                    if any(request.get('chat_template_kwargs') != {'enable_thinking': False} for request in observed):
                        raise ValueError('GPU-013 product did not forward frozen request options')
                    json.loads((evidence / 'failure.json').read_text(encoding='utf-8'))
                    g7_runner.wait_for_trace(trace)
                    events = [json.loads(line) for line in trace.read_text(encoding='utf-8').splitlines() if line.strip()]
                    if len({row['request'] for row in events if row.get('kind') == 'complete'}) != 9:
                        raise ValueError('GPU-013 proxy did not drain all requests')
                finally:
                    proxy.terminate()
                    try:
                        proxy.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        proxy.kill()
                        proxy.wait(timeout=10)
        if manifest_path.read_bytes() != raw:
            raise ValueError('frozen manifest changed during rehearsal')
        case_launcher.preflight(manifest)
    finally:
        server.shutdown()
        thread.join(timeout=10)
        server.server_close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', required=True, type=Path)
    args = parser.parse_args()
    try:
        rehearsal(args.manifest)
    except (ValueError, KeyError, TypeError, OSError, subprocess.SubprocessError) as exc:
        parser.exit(1, f'partial GPU-013 rehearsal failed: {exc}\n')
    print('Partial GPU-013 loopback passed; not certification evidence or a selected-set rehearsal.')


if __name__ == '__main__':
    main()
