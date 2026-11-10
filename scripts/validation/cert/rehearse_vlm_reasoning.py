#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Partial REASON-005 product/proxy loopback; never case evidence."""
import argparse
import base64
import hashlib
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
import manifest_tool
import rehearse_live_modalities
import v_runner

CASE = 'BC-REASON-005'


def rehearsal(manifest_path):
    manifest_path = Path(manifest_path).resolve(strict=True)
    raw = manifest_path.read_bytes()
    manifest = json.loads(raw, object_pairs_hook=manifest_tool.unique_object)
    if list(manifest.get('cases', {})) != [CASE] or manifest.get('lane') != 'V':
        raise ValueError('exactly the frozen REASON-005 V case is required')
    case_launcher.preflight(manifest)
    baseline = rehearse_live_modalities.parse_frozen_command(manifest, CASE, 'V')
    candidate = Path(manifest['candidate']['path']).resolve(strict=True)
    _, config = manifest_tool.load_manifest(manifest['lane_manifest']['path'], 'V', candidate, baseline, '0')
    observed = []

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_POST(self):
            if self.path != '/v1/chat/completions':
                self.send_error(404)
                return
            try:
                request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                observed.append(request)
                data = (b'data: {"choices":[{"index":0,"delta":{"reasoning_content":"synthetic thought"}}]}\n\n'
                        b'data: {"choices":[{"index":0,"delta":{"content":"synthetic answer"}}]}\n\n'
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
        with tempfile.TemporaryDirectory(prefix='cert-vlm-rehearsal-') as temporary:
            root = Path(temporary)
            evidence = root / 'evidence'
            evidence.mkdir()
            with socket.socket() as reserve:
                reserve.bind(('127.0.0.1', 0))
                proxy_port = reserve.getsockname()[1]
            args = SimpleNamespace(out=root, candidate=candidate, prompt=Path(config['prompt_sets']['default']['path']),
                                   model=config['model'], port=server.server_address[1], proxy_port=proxy_port)
            binary, image = v_runner.case_inputs(args, config)
            data = evidence / 'v-reasoning-requests.jsonl'
            command = v_runner.benchmark_command(args, binary, data, evidence)
            env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(root), 'LANG': 'C.UTF-8',
                   'LC_ALL': 'C.UTF-8', 'NO_PROXY': 'localhost,127.0.0.1', 'no_proxy': 'localhost,127.0.0.1',
                   'HF_HUB_OFFLINE': '1', 'TRANSFORMERS_OFFLINE': '1'}
            trace = g4_modality_runner.run_proxy(args, 'v-reasoning', command, image=True, env=env)
            rows = [json.loads(line) for line in data.read_text(encoding='utf-8').splitlines() if line.strip()]
            requests = [row for row in rows if '.request.' in row.get('schema_version', '') and row.get('phase') == 'measure']
            if len(requests) != 7 or len(observed) != 7 or any(row.get('error') is not None for row in requests):
                raise ValueError('REASON-005 did not complete seven product requests')
            for request in observed:
                if not isinstance(request.get('messages'), list) or any(not isinstance(m, dict) for m in request['messages']):
                    raise ValueError('REASON-005 wire messages are malformed')
                images = [part['image_url']['url'] for message in request['messages']
                          for part in (message.get('content') if isinstance(message.get('content'), list) else [])
                          if isinstance(part, dict) and part.get('type') == 'image_url']
                if len(images) != 1 or request.get('chat_template_kwargs') != {'enable_thinking': True}:
                    raise ValueError('REASON-005 wire image or thinking option is missing')
                header, separator, encoded = images[0].partition(',')
                if (not separator or not header.startswith('data:image/') or not header.endswith(';base64')
                        or hashlib.sha256(base64.b64decode(encoded, validate=True)).hexdigest() != manifest_tool.sha256_file(image)):
                    raise ValueError('REASON-005 wire image differs from frozen input')
            events = [json.loads(line) for line in trace.splitlines() if line.strip()]
            if len({row['request'] for row in events if row.get('kind') == 'complete'}) != 7:
                raise ValueError('REASON-005 proxy did not drain all requests')
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
        parser.exit(1, f'partial REASON-005 rehearsal failed: {exc}\n')
    print('Partial REASON-005 loopback passed; not certification evidence or a selected-set rehearsal.')


if __name__ == '__main__':
    main()
