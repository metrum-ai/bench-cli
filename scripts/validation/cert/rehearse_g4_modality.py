#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Partial GPU-011 LLM/VLM product-argv loopback, never case evidence."""
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
import r_runner
import rehearse_live_modalities

CASE = 'BC-GPU-011'


def rehearsal(manifest_path):
    manifest_path = Path(manifest_path).resolve(strict=True)
    raw = manifest_path.read_bytes()
    manifest = json.loads(raw, object_pairs_hook=manifest_tool.unique_object)
    if list(manifest.get('cases', {})) != [CASE] or manifest.get('lane') != 'G4':
        raise ValueError('exactly the frozen GPU-011 G4 case is required')
    case_launcher.preflight(manifest)
    baseline = rehearse_live_modalities.parse_frozen_command(manifest, CASE, 'G4')
    candidate = Path(manifest['candidate']['path']).resolve(strict=True)
    _, config = manifest_tool.load_manifest(manifest['lane_manifest']['path'], 'G4', candidate, baseline, '0')
    strategic, vlm_bin, image, vlm_prompt, vlm, _ = g4_modality_runner.modality_inputs(config, candidate)
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
        with tempfile.TemporaryDirectory(prefix='cert-g4-rehearsal-') as temporary:
            root = Path(temporary)
            evidence = root / 'evidence'
            evidence.mkdir()
            with socket.socket() as reserve:
                reserve.bind(('127.0.0.1', 0))
                proxy_port = reserve.getsockname()[1]
            args = SimpleNamespace(out=root, candidate=candidate, prompt=Path(config['prompt_sets']['default']['path']),
                                   model=config['model'], port=server.server_address[1], proxy_port=proxy_port)
            env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(root), 'LANG': 'C.UTF-8',
                   'LC_ALL': 'C.UTF-8', 'NO_PROXY': 'localhost,127.0.0.1', 'no_proxy': 'localhost,127.0.0.1',
                   'HF_HUB_OFFLINE': '1', 'TRANSFORMERS_OFFLINE': '1'}
            llm_ndjson = evidence / 'g4-modality-llm-telemetry.jsonl'
            data, trace, code, _ = r_runner.run_cell(args, 'g4-modality-llm', candidate, 4, 4096, True,
                ['--slo', 'e2e=600', '--ndjson', str(llm_ndjson)], concurrency=1, env=env)
            check_product(data, trace, code, 4)
            g4_modality_runner.strategic(args, strategic, 'g4-modality-llm-strategic', args.model, args.prompt, 'chat', env=env)
            data_path = evidence / 'g4-modality-vlm-requests.jsonl'
            telemetry = evidence / 'g4-modality-vlm-telemetry.jsonl'
            command = g4_modality_runner.vlm_command(args, vlm_bin, vlm_prompt, vlm['model'], data_path, telemetry)
            trace = g4_modality_runner.run_proxy(args, 'g4-modality-vlm', command, image=True, env=env)
            check_product(data_path.read_text(encoding='utf-8'), trace, 0, 4)
            g4_modality_runner.strategic(args, strategic, 'g4-modality-vlm-strategic', vlm['model'], vlm_prompt, 'vlm', env=env)
            if len(observed) != 16:
                raise ValueError('GPU-011 loopback did not forward all four product cohorts')
            for index, request in enumerate(observed):
                if request.get('chat_template_kwargs') != {'enable_thinking': True}:
                    raise ValueError('GPU-011 product did not forward thinking option')
                expected_model = args.model if index < 8 else vlm['model']
                if request.get('model') != expected_model:
                    raise ValueError('GPU-011 product did not forward pinned model')
                messages = request.get('messages')
                if not isinstance(messages, list) or any(not isinstance(m, dict) for m in messages):
                    raise ValueError('GPU-011 wire messages are malformed')
                images = [part['image_url']['url'] for message in messages
                          for part in (message.get('content') if isinstance(message.get('content'), list) else [])
                          if isinstance(part, dict) and part.get('type') == 'image_url']
                if index < 8:
                    if images:
                        raise ValueError('GPU-011 LLM cohort unexpectedly sent images')
                    continue
                if len(images) != 1:
                    raise ValueError('GPU-011 VLM cohort must send one image')
                header, separator, encoded = images[0].partition(',')
                if (not separator or not header.startswith('data:image/') or not header.endswith(';base64')
                        or hashlib.sha256(base64.b64decode(encoded, validate=True)).hexdigest() != manifest_tool.sha256_file(image)):
                    raise ValueError('GPU-011 wire image differs from frozen input')
        if manifest_path.read_bytes() != raw:
            raise ValueError('frozen manifest changed during rehearsal')
        case_launcher.preflight(manifest)
    finally:
        server.shutdown()
        thread.join(timeout=10)
        server.server_close()


def check_product(data, trace, code, expected):
    rows = [json.loads(line) for line in data.splitlines() if line.strip()]
    requests = [row for row in rows if '.request.' in row.get('schema_version', '') and row.get('phase') == 'measure']
    events = [json.loads(line) for line in trace.splitlines() if line.strip()]
    if (code or len(requests) != expected or any(row.get('error') is not None for row in requests)
            or len({row['request'] for row in events if row.get('kind') == 'complete'}) != expected):
        raise ValueError('GPU-011 product/proxy did not complete its request cohort')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', required=True, type=Path)
    args = parser.parse_args()
    try:
        rehearsal(args.manifest)
    except (ValueError, KeyError, TypeError, OSError, subprocess.SubprocessError) as exc:
        parser.exit(1, f'partial GPU-011 rehearsal failed: {exc}\n')
    print('Partial GPU-011 loopback passed; not certification evidence or a selected-set rehearsal.')


if __name__ == '__main__':
    main()
