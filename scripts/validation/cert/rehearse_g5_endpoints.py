#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Partial GPU-015 multi-endpoint parser/argv loopback, never case evidence."""
import argparse
import contextlib
import json
import os
import socket
import subprocess
import sys
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from types import SimpleNamespace

import case_launcher
import g5_endpoints_runner
import g6_counter_runner
import g7_runner
import l1_runner
import manifest_tool
import rehearse_live_modalities

CASE = 'BC-GPU-015'
ROOT = Path(__file__).resolve().parents[3]


def rehearsal(manifest_path):
    manifest_path = Path(manifest_path).resolve(strict=True)
    raw = manifest_path.read_bytes()
    manifest = json.loads(raw, object_pairs_hook=manifest_tool.unique_object)
    if list(manifest.get('cases', {})) != [CASE] or manifest.get('lane') != 'G5':
        raise ValueError('exactly the frozen GPU-015 G5 case is required')
    case_launcher.preflight(manifest)
    baseline = rehearse_live_modalities.parse_frozen_command(manifest, CASE, 'G5')
    candidate = Path(manifest['candidate']['path']).resolve(strict=True)
    _, config = manifest_tool.load_manifest(manifest['lane_manifest']['path'], 'G5', candidate, baseline, '0', case=CASE)
    g5_endpoints_runner.configuration(config)
    observed = {'survivor': [], 'victim': []}

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_POST(self):
            if self.path != '/v1/chat/completions':
                self.send_error(404)
                return
            try:
                request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                observed[self.server.label].append(request)
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

    servers = []
    threads = []
    try:
        for label in ('survivor', 'victim'):
            server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
            server.label = label
            servers.append(server)
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            threads.append(thread)
            thread.start()
        with tempfile.TemporaryDirectory(prefix='cert-g5-endpoint-rehearsal-') as temporary, contextlib.ExitStack() as stack:
            root = Path(temporary)
            evidence = root / 'evidence'
            evidence.mkdir()
            with socket.socket() as one, socket.socket() as two:
                one.bind(('127.0.0.1', 0))
                two.bind(('127.0.0.1', 0))
                ports = [one.getsockname()[1], two.getsockname()[1]]
            env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(root), 'LANG': 'C.UTF-8',
                   'LC_ALL': 'C.UTF-8', 'NO_PROXY': 'localhost,127.0.0.1', 'no_proxy': 'localhost,127.0.0.1',
                   'HF_HUB_OFFLINE': '1', 'TRANSFORMERS_OFFLINE': '1'}
            proxies = []
            traces = []
            try:
                for index, label in enumerate(('survivor', 'victim')):
                    trace = evidence / f'g5-endpoints-{label}-trace.jsonl'
                    traces.append(trace)
                    err = stack.enter_context((evidence / f'{label}-proxy-error.txt').open('wb'))
                    proxy = subprocess.Popen([sys.executable, str(Path(__file__).with_name('trace_proxy.py')),
                        '--listen-port', str(ports[index]), '--upstream-port', str(servers[index].server_address[1]),
                        '--out', str(trace)], stdout=subprocess.DEVNULL, stderr=err, cwd=ROOT, env=env)
                    proxies.append(proxy)
                    g6_counter_runner.wait_for_proxy(ports[index], proxy)
                endpoints = evidence / 'g5-endpoints.yaml'
                endpoints.write_text(g5_endpoints_runner.endpoints_yaml(*ports), encoding='utf-8')
                data = evidence / 'g5-endpoints-requests.jsonl'
                args = SimpleNamespace(candidate=candidate, prompt=Path(config['prompt_sets']['default']['path']), model=config['model'])
                command = g5_endpoints_runner.benchmark_command(args, endpoints, data, evidence)
                code = l1_runner.call(command, evidence / 'stdout.txt', evidence / 'stderr.txt', 1800, env=env)
                rows = [json.loads(line) for line in data.read_text(encoding='utf-8').splitlines() if line.strip()]
                requests = [row for row in rows if '.request.' in row.get('schema_version', '') and row.get('phase') == 'measure']
                if (code or len(requests) != 64 or any(row.get('error') is not None for row in requests)
                        or {row.get('endpoint') for row in requests} != set(observed)
                        or sum(len(rows) for rows in observed.values()) != 64 or any(not rows for rows in observed.values())
                        or any(request.get('chat_template_kwargs') != {'enable_thinking': False}
                               for values in observed.values() for request in values)):
                    raise ValueError(f'GPU-015 loopback incomplete: exit={code}, measured={len(requests)}, '
                                     f'survivor={len(observed["survivor"])}, victim={len(observed["victim"])}')
                for label, trace in zip(('survivor', 'victim'), traces):
                    g7_runner.wait_for_trace(trace)
                    events = [json.loads(line) for line in trace.read_text(encoding='utf-8').splitlines() if line.strip()]
                    if len({row['request'] for row in events if row.get('kind') == 'complete'}) != len(observed[label]):
                        raise ValueError(f'GPU-015 {label} proxy did not drain all requests')
            finally:
                for proxy in proxies:
                    if proxy.poll() is None:
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
        for server in servers:
            server.shutdown()
        for thread in threads:
            thread.join(timeout=10)
        for server in servers:
            server.server_close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', required=True, type=Path)
    args = parser.parse_args()
    try:
        rehearsal(args.manifest)
    except (ValueError, KeyError, TypeError, OSError, subprocess.SubprocessError) as exc:
        parser.exit(1, f'partial GPU-015 rehearsal failed: {exc}\n')
    print('Partial GPU-015 loopback passed; not certification evidence or a selected-set rehearsal.')


if __name__ == '__main__':
    main()
