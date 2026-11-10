#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Partial COMP-002 candidate-side loopback; reference parser is NOT run."""
import argparse
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
import comp_reference_runner
import manifest_tool
import rehearse_live_modalities

CASE = 'BC-COMP-002'


def rehearsal(manifest_path, cache):
    manifest_path = Path(manifest_path).resolve(strict=True)
    raw = manifest_path.read_bytes()
    manifest = json.loads(raw, object_pairs_hook=manifest_tool.unique_object)
    if list(manifest.get('cases', {})) != [CASE] or manifest.get('lane') != 'D':
        raise ValueError('exactly the frozen COMP-002 D case is required')
    case_launcher.preflight(manifest)
    baseline = rehearse_live_modalities.parse_frozen_command(manifest, CASE, 'D')
    candidate = Path(manifest['candidate']['path']).resolve(strict=True)
    _, config = manifest_tool.load_manifest(manifest['lane_manifest']['path'], 'D', candidate, baseline, '0')
    prompts, sharegpt, order_path, order = comp_reference_runner.reference_inputs(config)
    snapshot = Path(cache).resolve(strict=True) / f"models--{config['model'].replace('/', '--')}" / 'snapshots' / config['revision']
    tokenizer = snapshot / 'tokenizer.json'
    if not tokenizer.is_file() or manifest_tool.sha256_file(tokenizer) != manifest['tokenizer']['sha256']:
        raise ValueError('reference tokenizer snapshot does not match frozen tokenizer pin')
    observed = []

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_POST(self):
            if self.path != '/v1/chat/completions':
                self.send_error(404)
                return
            try:
                observed.append(json.loads(self.rfile.read(int(self.headers['Content-Length']))))
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
        with tempfile.TemporaryDirectory(prefix='cert-comp-rehearsal-') as temporary:
            root = Path(temporary)
            evidence = root / 'evidence'
            evidence.mkdir()
            with socket.socket() as reserve:
                reserve.bind(('127.0.0.1', 0))
                proxy_port = reserve.getsockname()[1]
            args = SimpleNamespace(out=root, candidate=candidate, model=config['model'], revision=config['revision'],
                                   image=config['image'], port=server.server_address[1], proxy_port=proxy_port)
            command, reference, data = comp_reference_runner.invocations(args, prompts, sharegpt, order_path,
                                                                           Path(cache).resolve(), 'rehearsal-not-run', evidence)
            if reference[:3] != ['docker', 'run', '--name'] or reference[reference.index('--pull') + 1] != 'never':
                raise ValueError('reference command does not retain offline pull policy')
            env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(root), 'LANG': 'C.UTF-8',
                   'LC_ALL': 'C.UTF-8', 'NO_PROXY': 'localhost,127.0.0.1', 'no_proxy': 'localhost,127.0.0.1',
                   'HF_HUB_OFFLINE': '1', 'TRANSFORMERS_OFFLINE': '1'}
            code, trace_path = comp_reference_runner.run_recorded(args, 'candidate', command, env=env)
            if code or not data.is_file():
                raise ValueError(f'COMP-002 candidate product argv failed: exit={code}')
            rows = [json.loads(line) for line in data.read_text(encoding='utf-8').splitlines() if line.strip()]
            requests = [row for row in rows if '.request.' in row.get('schema_version', '') and row.get('phase') == 'measure']
            events = [json.loads(line) for line in trace_path.read_text(encoding='utf-8').splitlines() if line.strip()]
            if (code or len(requests) != 64 or len(observed) != 64 or any(row.get('error') is not None for row in requests)
                    or len({row['request'] for row in events if row.get('kind') == 'complete'}) != 64):
                raise ValueError(f'COMP-002 candidate loopback incomplete: exit={code}, measured={len(requests)}, upstream={len(observed)}')
            for request, slot in zip(observed, order):
                messages = request.get('messages')
                if not isinstance(messages, list) or len(messages) != 1 or messages[0].get('role') != 'user':
                    raise ValueError('COMP-002 candidate wire prompt shape differs')
                text = messages[0].get('content')
                if not isinstance(text, str) or hashlib.sha256(text.encode()).hexdigest() != slot['prompt_sha256']:
                    raise ValueError('COMP-002 candidate wire prompt order differs from frozen reference')
                if (request.get('model') != args.model or request.get('max_tokens') != 1024 or request.get('seed') != 42
                        or request.get('chat_template_kwargs') != {'enable_thinking': True}):
                    raise ValueError('COMP-002 candidate wire options differ from frozen comparison')
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
    parser.add_argument('--tokenizer-cache', required=True, type=Path)
    args = parser.parse_args()
    try:
        rehearsal(args.manifest, args.tokenizer_cache)
    except (ValueError, KeyError, TypeError, OSError, subprocess.SubprocessError) as exc:
        parser.exit(1, f'partial COMP-002 candidate rehearsal failed: {exc}\n')
    print('Partial COMP-002 candidate loopback passed; reference parser NOT RUN; not a complete rehearsal.')


if __name__ == '__main__':
    main()
