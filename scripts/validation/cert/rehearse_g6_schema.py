#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Partial GPU-016 offline schema and product-argv rehearsal; never case evidence."""
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
import g6_schema_runner
import l1_runner
import manifest_tool
import r_runner
import rehearse_live_modalities

CASE = 'BC-GPU-016'


def rehearsal(manifest_path):
    manifest_path = Path(manifest_path).resolve(strict=True)
    raw = manifest_path.read_bytes()
    manifest = json.loads(raw, object_pairs_hook=manifest_tool.unique_object)
    if list(manifest.get('cases', {})) != [CASE] or manifest.get('lane') != 'G6':
        raise ValueError('exactly the frozen GPU-016 G6 case is required')
    case_launcher.preflight(manifest)
    baseline = rehearse_live_modalities.parse_frozen_command(manifest, CASE, 'G6')
    candidate = Path(manifest['candidate']['path']).resolve(strict=True)
    _, config = manifest_tool.load_manifest(manifest['lane_manifest']['path'], 'G6', candidate, baseline, '0')
    strategic = g4_modality_runner.pinned_file(config, 'strategic_binary', 'metrum-ai-bench-cli-strategic', candidate)
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
        with tempfile.TemporaryDirectory(prefix='cert-g6-rehearsal-') as temporary:
            root = Path(temporary)
            evidence = root / 'evidence'
            evidence.mkdir()
            with socket.socket() as reserve:
                reserve.bind(('127.0.0.1', 0))
                proxy_port = reserve.getsockname()[1]
            args = SimpleNamespace(out=root, candidate=candidate, strategic=strategic,
                                   prompt=Path(config['prompt_sets']['default']['path']), model=config['model'],
                                   port=server.server_address[1], proxy_port=proxy_port)
            env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(root), 'LANG': 'C.UTF-8',
                   'LC_ALL': 'C.UTF-8', 'NO_PROXY': 'localhost,127.0.0.1', 'no_proxy': 'localhost,127.0.0.1',
                   'HF_HUB_OFFLINE': '1', 'TRANSFORMERS_OFFLINE': '1', 'CARGO_NET_OFFLINE': 'true',
                   'CARGO_HOME': os.environ.get('CARGO_HOME', str(Path.home() / '.cargo'))}
            for name in ('RUSTUP_HOME', 'RUSTUP_TOOLCHAIN', 'CARGO_TARGET_DIR'):
                if os.environ.get(name):
                    env[name] = os.environ[name]
            fixtures = l1_runner.call(g6_schema_runner.fixture_command(), evidence / 'fixtures.txt',
                                      evidence / 'fixtures-error.txt', 1800, env=env)
            if fixtures:
                raise ValueError('GPU-016 offline schema fixtures failed; prepare the actual host cache')
            llm_ndjson = evidence / 'g6-schema-llm-telemetry.jsonl'
            data, trace, code, _ = r_runner.run_cell(args, 'g6-schema-llm', candidate, 16, 4096, True,
                ['--ndjson', str(llm_ndjson)], env=env)
            rows = [json.loads(line) for line in data.splitlines() if line.strip()]
            requests = [row for row in rows if '.request.' in row.get('schema_version', '') and row.get('phase') == 'measure']
            events = [json.loads(line) for line in trace.splitlines() if line.strip()]
            if (code or len(requests) != 16 or any(row.get('error') is not None for row in requests)
                    or len({row['request'] for row in events if row.get('kind') == 'complete'}) != 16):
                raise ValueError('GPU-016 LLM product/proxy request cohort failed')
            command, csv_path, ndjson, html = g6_schema_runner.strategic_command(args, evidence)
            code = l1_runner.call(command, evidence / 'strategic.json', evidence / 'strategic-error.txt', 1800, env=env)
            if code or len(observed) != 24 or not all(path.is_file() for path in (csv_path, ndjson, html)):
                raise ValueError(f'GPU-016 strategic argv failed: exit={code}, requests={len(observed)}')
            json.loads((evidence / 'strategic.json').read_text(encoding='utf-8'))
            if any(request.get('chat_template_kwargs') != {'enable_thinking': True} or request.get('model') != args.model
                   for request in observed):
                raise ValueError('GPU-016 model or thinking option did not reach the wire')
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
        parser.exit(1, f'partial GPU-016 rehearsal failed: {exc}\n')
    print('Partial GPU-016 loopback passed; not certification evidence or a selected-set rehearsal.')


if __name__ == '__main__':
    main()
