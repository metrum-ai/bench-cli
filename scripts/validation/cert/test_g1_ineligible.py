# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""CPU regression: no source content is synthesized or published as GPU evidence."""
import base64
import hashlib
import http.client
import json
import os
import socket
import tempfile
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from types import SimpleNamespace

import g1_clock_checks
import g1_variants_checks
import g1_variants_runner
import trace_proxy
from test_sse_reorder import frame

DONE = b'data: [DONE]\n\n'
TERMINAL = b'data: {"choices":[{"index":0,"delta":{},"finish_reason":"length"}]}\n\n'
INELIGIBLE = b''.join([frame({'role': 'assistant'}), frame({'reasoning': 'thinking'}), TERMINAL, DONE])
STOP = b'data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}\n\n'
ELIGIBLE = b''.join([frame({'role': 'assistant'}), frame({'reasoning': 'thinking'}), frame({'content': 'answer'}), STOP, DONE])


def proxy_replay(bodies, mode):
    class Upstream(BaseHTTPRequestHandler):
        count = 0
        def log_message(self, *args): pass
        def do_POST(self):
            self.rfile.read(int(self.headers['Content-Length']))
            body = bodies[type(self).count]; type(self).count += 1
            self.send_response(200); self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Content-Length', str(len(body))); self.end_headers(); self.wfile.write(body)
    server = ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
    thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
    try:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'trace'; recorder = trace_proxy.Recorder(path)
            proxy = ThreadingHTTPServer(('127.0.0.1', 0), trace_proxy.make_handler(server.server_port, recorder, reorder=mode))
            worker = threading.Thread(target=proxy.serve_forever, daemon=True); worker.start()
            try:
                responses = []
                for _ in bodies:
                    conn = http.client.HTTPConnection('127.0.0.1', proxy.server_port, timeout=30)
                    conn.request('POST', '/v1/chat/completions', body=b'{}')
                    response = conn.getresponse(); responses.append((response.status, response.read())); conn.close()
            finally:
                proxy.shutdown(); proxy.server_close(); worker.join(timeout=10); recorder.stream.close()
            return responses, path.read_text()
    finally:
        server.shutdown(); server.server_close(); thread.join(timeout=10)


def candidate_replay(bodies, mode):
    class Upstream(BaseHTTPRequestHandler):
        count = 0
        def log_message(self, *args): pass
        def do_POST(self):
            self.rfile.read(int(self.headers['Content-Length']))
            body = bodies[type(self).count]; type(self).count += 1
            self.send_response(200); self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Content-Length', str(len(body))); self.end_headers()
            first, rest = trace_proxy.take_frame(body)
            self.wfile.write(first); self.wfile.flush(); time.sleep(.02)
            self.wfile.write(rest)
    server = ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
    thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
    try:
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp); (out / 'evidence').mkdir()
            prompt = out / 'prompts.jsonl'; prompt.write_text('{"prompt":"hello"}\n')
            with socket.socket() as sock:
                sock.bind(('127.0.0.1', 0)); proxy_port = sock.getsockname()[1]
            args = SimpleNamespace(candidate=Path(os.environ['CERT_TEST_LLM_BIN']), out=out,
                                   model='fixture', port=server.server_port, proxy_port=proxy_port)
            return g1_variants_runner.run_cell(args, 'replay', prompt, {'chat_template_kwargs': {'enable_thinking': True}}, mode)
    finally:
        server.shutdown(); server.server_close(); thread.join(timeout=10)


def rows_for(trace):
    # Derived clock fixtures exercise checker logic only; not independent timing evidence.
    rows = []
    for i, stream in enumerate(g1_clock_checks.stream_clocks(trace, 'forwarded_chunk')):
        row = {'schema_version': 'metrum-ai-bench-cli.request.v5', 'seq': i, 'phase': 'measure',
               'latency_s': (stream['finish'] - stream['send']) / 1e9,
               'ttft_s': (stream['first'] - stream['send']) / 1e9, 'ttft_source': 'stream',
               'ttft_visible_answer_s': (stream['visible'] - stream['send']) / 1e9 if stream['visible'] else None}
        if stream['visible'] is None:
            row.update(outcome='reasoning_truncated', answer_completed=False, finish_reason='length')
        else:
            row.update(outcome='answer', answer_completed=True, finish_reason='stop')
        rows.append(json.dumps(row))
    return '\n'.join(rows)


class IneligibleTests(unittest.TestCase):
    def test_proxy_passthrough_and_required_population(self):
        for mode, required in [('answer-first', 'answer_before_reasoning'), ('same-event', 'same_event')]:
            with self.subTest(mode=mode):
                responses, trace = proxy_replay([ELIGIBLE, INELIGIBLE], mode)
                self.assertEqual(responses[1], (200, INELIGIBLE))
                events = [json.loads(x) for x in trace.splitlines() if json.loads(x)['request'] == 2]
                self.assertEqual([x['kind'] for x in events].count('ineligible'), 1)
                self.assertNotIn('error', [x['kind'] for x in events])
                result = g1_variants_checks.evaluate(rows_for(trace), trace, 0, mode, required)
                self.assertEqual(result['populations']['reasoning_truncated'], 1)
                self.assertEqual(result['populations']['transformed'], 1)
                bad = rows_for(trace).replace('reasoning_truncated', 'answer')
                with self.assertRaises(ValueError): g1_variants_checks.evaluate(bad, trace, 0, mode, required)
                _, zero = proxy_replay([INELIGIBLE], mode)
                with self.assertRaises(ValueError): g1_variants_checks.evaluate(rows_for(zero), zero, 0, mode, required)
                _, unfinished = proxy_replay([ELIGIBLE.replace(STOP, TERMINAL), INELIGIBLE], mode)
                with self.assertRaisesRegex(ValueError, 'complete answer/tool'):
                    g1_variants_checks.evaluate(rows_for(unfinished), unfinished, 0, mode, required)

    def test_unselected_first_also_forwards_missing_selected_output(self):
        responses, trace = proxy_replay([INELIGIBLE], 'unselected-first')
        self.assertEqual(responses[0], (200, INELIGIBLE))
        events = [json.loads(x) for x in trace.splitlines()]
        self.assertEqual([e['kind'] for e in events].count('ineligible'), 1)
        self.assertNotIn('error', [e['kind'] for e in events])


@unittest.skipUnless(os.environ.get('CERT_G1_ATTEMPT3_EVIDENCE') and os.environ.get('CERT_TEST_LLM_BIN'),
                     'private exact captured replay requires evidence and frozen binary')
class CapturedReplayTests(unittest.TestCase):
    def test_exact_attempt3_through_proxy_candidate_and_checker(self):
        root = Path(os.environ['CERT_G1_ATTEMPT3_EVIDENCE'])
        hashes = {'reasoning': '3fc909abd68ab425983b9a41acca0aa2f43d61d967d99b9c451f8b64c915c29d',
                  'answer-first': '078b629ff8fb2d76081e032ac90b0569dced0d11d5ebb6a2020074c917770036',
                  'same-event': 'f5b7db0417655732503264ee0c248ed2fdccd7f630d25b129d38911bb8a41bbc'}
        for cell, expected_hash in hashes.items():
            with self.subTest(cell=cell):
                events = [json.loads(x) for x in (root / f'g1-variants-{cell}-trace.jsonl').read_text().splitlines()]
                bodies = [b''.join(base64.b64decode(e['bytes_b64']) for e in events
                                   if e['request'] == i and e['kind'] == 'chunk') for i in range(1, 5)]
                self.assertEqual(hashlib.sha256(bodies[3]).hexdigest(), expected_hash)
                mode = None if cell == 'reasoning' else cell
                data, trace, code = candidate_replay(bodies, mode)
                replay_events = [json.loads(x) for x in trace.splitlines() if json.loads(x)['request'] == 4]
                forwarded = b''.join(base64.b64decode(e['bytes_b64']) for e in replay_events if e['kind'] == 'forwarded_chunk')
                self.assertEqual(forwarded, bodies[3])
                self.assertEqual(code, 0)
                required = {'reasoning': 'reasoning', 'answer-first': 'answer_before_reasoning', 'same-event': 'same_event'}[cell]
                result = g1_variants_checks.evaluate(data, trace, code, mode, required)
                self.assertEqual(result['populations']['reasoning_truncated'], 1)
                if cell != 'reasoning': self.assertEqual(result['populations']['transformed'], 3)


if __name__ == '__main__':
    unittest.main()
