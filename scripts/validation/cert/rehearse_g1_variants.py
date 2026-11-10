#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Partial GPU-003 runner/CLI rehearsal using an isolated synthetic loopback.

This exercises real command assembly, proxy and product parsing with disposable
outputs. It does not run validators, write case records, prove a live server or
replace the required complete selected-set and native-offload rehearsal.
"""
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
import g1_variants_runner
import manifest_tool
import rehearse_live_modalities

CASE = 'BC-GPU-003'


def rehearsal(manifest_path):
    manifest_path = Path(manifest_path).resolve(strict=True)
    raw = manifest_path.read_bytes()
    manifest = json.loads(raw, object_pairs_hook=manifest_tool.unique_object)
    if list(manifest.get('cases', {})) != [CASE] or manifest.get('lane') != 'G1':
        raise ValueError('exactly the frozen GPU-003 G1 case is required')
    case_launcher.preflight(manifest)
    baseline = rehearse_live_modalities.parse_frozen_command(manifest, CASE, 'G1')
    candidate = Path(manifest['candidate']['path']).resolve(strict=True)
    _, config = manifest_tool.load_manifest(manifest['lane_manifest']['path'], 'G1', candidate, baseline, '0')
    observed = []

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_POST(self):
            if self.path != '/v1/chat/completions':
                self.send_error(404)
                return
            try:
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                observed.append(body)
                events = [{'choices': [{'index': 0, 'delta': {'reasoning_content': 'synthetic thought'}}]},
                          {'choices': [{'index': 0, 'delta': {'content': 'synthetic answer'}}]}]
                if body.get('n') == 2:
                    events.append({'choices': [{'index': 1, 'delta': {'content': 'synthetic alternative'}}]})
                events.append({'choices': [{'index': 0, 'delta': {}, 'finish_reason': 'stop'}],
                               'usage': {'prompt_tokens': 1, 'completion_tokens': 1, 'total_tokens': 2}})
                response = b''.join(b'data: ' + json.dumps(event).encode() + b'\n\n' for event in events) + b'data: [DONE]\n\n'
            except (ValueError, TypeError, KeyError):
                self.send_error(400)
                return
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Content-Length', str(len(response)))
            self.end_headers()
            self.wfile.write(response)

    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix='cert-g1-rehearsal-') as temporary:
            root = Path(temporary)
            (root / 'evidence').mkdir()
            # The proxy is an owned local process; a bind race fails rehearsal.
            with socket.socket() as reserve:
                reserve.bind(('127.0.0.1', 0))
                proxy_port = reserve.getsockname()[1]
            args = SimpleNamespace(out=root, candidate=candidate, prompt=Path(config['prompt_sets']['default']['path']),
                                   model=config['model'], port=server.server_address[1], proxy_port=proxy_port)
            env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(root),
                   'LANG': 'C.UTF-8', 'LC_ALL': 'C.UTF-8', 'NO_PROXY': 'localhost,127.0.0.1',
                   'no_proxy': 'localhost,127.0.0.1', 'HF_HUB_OFFLINE': '1', 'TRANSFORMERS_OFFLINE': '1'}
            cells = g1_variants_runner.modes(args, config)
            if len(cells) != 6:
                raise ValueError('GPU-003 cell selection changed')
            for label, prompt, body, reorder, _ in cells:
                before = len(observed)
                data, trace, code = g1_variants_runner.run_cell(args, 'g1-variants-' + label, prompt, body, reorder, env=env)
                rows = [json.loads(line) for line in data.splitlines() if line.strip()]
                requests = [row for row in rows if '.request.' in row.get('schema_version', '')
                            and row.get('phase') == 'measure']
                wire = observed[before:]
                if (code or len(wire) != 4 or len(requests) != 4 or not trace
                        or any(row.get('error') is not None for row in requests)
                        or any(request.get(key) != value for request in wire for key, value in body.items())):
                    raise ValueError(f'GPU-003 {label} loopback incomplete: exit={code}, '
                                     f'upstream={len(wire)}, measured={len(requests)}, trace={bool(trace)}')
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
        parser.exit(1, f'partial GPU-003 rehearsal failed: {exc}\n')
    print('Partial GPU-003 loopback passed; not certification evidence or a selected-set rehearsal.')


if __name__ == '__main__':
    main()
