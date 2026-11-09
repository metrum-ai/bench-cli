#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Exercise frozen L1/L2/L3/L4 smoke argv and real product parsing on loopback.

This is a *partial* offline check, not the complete selected-case rehearsal,
not a network sandbox, and never a GPU run or certification evidence. It
creates only temporary outputs and never invokes a case record writer.
"""
import argparse
import base64
import json
import os
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from types import SimpleNamespace

import case_launcher
import derived_inputs
import freeze_case
import generated_inputs
import l1_runner
import live_modality_runner
import manifest_tool

ROOT = Path(__file__).resolve().parents[3]
MODALITIES = {'BC-LIVE-001': ('L1', 'llm'), 'BC-LIVE-002': ('L2', 'vlm'),
              'BC-LIVE-003': ('L3', 'asr'), 'BC-LIVE-004': ('L4', 'imagegen')}
# Valid tiny PNG; this is only a fake response for the CPU-only rehearsal.
PNG = base64.b64encode((ROOT / 'test-data/vlm/shapes-512.png').read_bytes()).decode('ascii')


def parse_frozen_command(manifest, case):
    entry = manifest['cases'][case]
    argv = entry['command']
    lane, _ = MODALITIES[case]
    expected = ['--case', case, '--lane', lane.lower()]
    if (Path(argv[0]).resolve() != Path(__file__).with_name('case_entry.sh').resolve()
            or argv[1:5] != expected or len(argv) != 13):
        raise ValueError('live case must use the exact frozen case-entry argv')
    fields = {}
    for index in range(5, len(argv), 2):
        key, value = argv[index:index + 2]
        if key not in ('--candidate', '--baseline', '--manifest', '--out') or key in fields:
            raise ValueError('invalid frozen lane argument list')
        fields[key] = value
    if set(fields) != {'--candidate', '--baseline', '--manifest', '--out'}:
        raise ValueError('missing frozen lane arguments')
    if (Path(fields['--candidate']).resolve() != Path(manifest['candidate']['path']).resolve()
            or Path(fields['--manifest']).resolve() != Path(manifest['lane_manifest']['path']).resolve()
            or fields['--out'] != entry['out']):
        raise ValueError('frozen argv differs from candidate, lane manifest or case output')
    baseline = Path(fields['--baseline']).resolve(strict=True)
    pins = {Path(item['path']).resolve(): item['sha256'] for item in manifest['inputs']}
    if baseline not in pins or manifest_tool.sha256_file(baseline) != pins[baseline]:
        raise ValueError('baseline must be among frozen input pins')
    for source in freeze_case.harness_sources(Path(__file__).parent):
        if source not in pins or manifest_tool.sha256_file(source) != pins[source]:
            raise ValueError('frozen harness source pin missing or changed')
    return baseline


def rehearsal(manifest_path, case):
    manifest_path = Path(manifest_path).resolve(strict=True)
    manifest_bytes = manifest_path.read_bytes()
    manifest = json.loads(manifest_bytes, object_pairs_hook=manifest_tool.unique_object)
    if case not in MODALITIES or list(manifest.get('cases', {})) != [case]:
        raise ValueError('exactly one supported frozen live modality case is required')
    case_launcher.preflight(manifest)
    baseline = parse_frozen_command(manifest, case)
    lane, modality = MODALITIES[case]
    candidate = Path(manifest['candidate']['path']).resolve(strict=True)
    _, config = manifest_tool.load_manifest(manifest['lane_manifest']['path'], lane, candidate, baseline, '0')
    model = config['model']
    expected_count = 16 if modality == 'llm' else live_modality_runner.CASES[modality][1]
    observed = []
    sse = (b'data: {"choices":[{"index":0,"delta":{"content":"synthetic answer"}}]}\n\n'
           b'data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],'
           b'"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}\n\n'
           b'data: [DONE]\n\n')

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_GET(self):
            if self.path != '/v1/models':
                self.send_error(404)
                return
            self.send_response(200)
            payload = json.dumps({'data': [{'id': model}]}).encode()
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

        def do_POST(self):
            length = int(self.headers.get('Content-Length', '0'))
            self.rfile.read(length)
            observed.append(self.path)
            if modality in ('llm', 'vlm') and self.path == '/v1/chat/completions':
                payload, content_type = sse, 'text/event-stream'
            elif modality == 'asr' and self.path == '/v1/audio/transcriptions':
                payload, content_type = b'{"text":"synthetic transcription"}', 'application/json'
            elif modality == 'imagegen' and self.path == '/v1/images/generations':
                payload = json.dumps({'created': 1, 'data': [{'b64_json': PNG}]}).encode()
                content_type = 'application/json'
            else:
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header('Content-Type', content_type)
            self.send_header('Content-Length', str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    server.daemon_threads = True
    worker = threading.Thread(target=server.serve_forever, kwargs={'poll_interval': .05}, daemon=True)
    worker.start()
    try:
        with tempfile.TemporaryDirectory(prefix='cert-live-rehearsal-') as temp:
            root = Path(temp)
            args = SimpleNamespace(modality=modality, candidate=candidate, prompt=Path(config['prompt_sets']['default']['path']),
                                   sut=Path(config['sut']['path']), model=model, port=server.server_port)
            output = root / 'output'
            if modality == 'llm':
                command, additions = l1_runner.smoke_invocation(args, output)
                headline = ['cargo', '--locked', 'xtask', 'assert-headline', 'llm', str(output / 'results.jsonl')]
            else:
                command, headline, additions = live_modality_runner.smoke_invocations(args, output)
            env = {'PATH': os.environ.get('PATH', ''), 'HOME': str(root), 'NO_PROXY': '*', 'no_proxy': '*',
                   'CARGO_HOME': os.environ.get('CARGO_HOME', str(Path.home() / '.cargo')),
                   'CARGO_NET_OFFLINE': 'true', 'HF_HUB_OFFLINE': '1', 'TRANSFORMERS_OFFLINE': '1',
                   'TOKENIZERS_PARALLELISM': 'false'}
            for name in ('CARGO_HOME', 'RUSTUP_HOME', 'RUSTUP_TOOLCHAIN'):
                if name in os.environ:
                    env[name] = os.environ[name]
            env.update(additions)
            for argv in (command, headline):
                result = subprocess.run(argv, cwd=ROOT, env=env, capture_output=True, text=True, timeout=1800)
                if result.returncode:
                    raise ValueError(f'live {modality} command failed (exit {result.returncode}): {result.stderr[-1000:]}')
            if modality == 'llm':
                # The real certification boundary also enforces the frozen case
                # output path. This rehearsal uses temporary output instead: it
                # does not spoof or invoke that boundary or publish records.
                # Independently exercise its same recipe and argv checks here.
                recipe = derived_inputs.resolved_recipe(manifest)
                prompt_input = output / 'prompts.jsonl'
                derived_inputs.check_command(recipe, output / 'prompt-command.json', prompt_input)
                binding = generated_inputs.materialize(recipe, manifest['build_receipt']['path'],
                    manifest['candidate_sha'], root / 'independent-derived', generated_inputs.contract_hash(recipe))
                if manifest_tool.sha256_file(prompt_input) != binding['actual']['prompts.jsonl']['sha256']:
                    raise ValueError('live L1 selector differs from frozen independent re-derivation')
            data = output / 'results.jsonl'
            rows = [json.loads(line) for line in data.read_text(encoding='utf-8').splitlines() if line.strip()]
            requests = [row for row in rows if '.request.' in row.get('schema_version', '') and row.get('phase') == 'measure']
            summaries = [row for row in rows if '.summary.' in row.get('schema_version', '')]
            if (len(requests) != expected_count or len(observed) != expected_count or len(summaries) != 1
                    or any(row.get('error') is not None for row in requests)
                    or summaries[0].get('config', {}).get('common', {}).get('require_sut') is not True):
                raise ValueError('live rehearsal did not parse SUT and complete its expected local requests')
            original = json.loads(args.sut.read_text(encoding='utf-8'), object_pairs_hook=manifest_tool.unique_object)
            captured = summaries[0].get('sut') or {}
            for section, keys in (('gpu', ('model', 'count', 'memory_gb')), ('model', ('id', 'revision'))):
                if any(captured.get(section, {}).get(key) != original.get(section, {}).get(key) for key in keys):
                    raise ValueError('live rehearsal summary SUT identity differs from frozen input')
        if manifest_path.read_bytes() != manifest_bytes:
            raise ValueError('frozen manifest changed during rehearsal')
        case_launcher.preflight(manifest)  # Recheck every artifact and harness pin after execution.
    finally:
        server.shutdown()
        server.server_close()
        worker.join(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--case', choices=sorted(MODALITIES), required=True)
    args = parser.parse_args()
    try:
        rehearsal(args.manifest, args.case)
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as exc:
        parser.exit(1, f'live modality rehearsal failed: {exc}\n')
    print(f'{args.case}: live smoke/parser loopback passed; NOT a full selected-case rehearsal')


if __name__ == '__main__':
    main()
