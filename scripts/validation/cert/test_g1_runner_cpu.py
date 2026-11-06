# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Optional CPU-only end-to-end collector test; never emits a case result."""
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

import g1_variants_checks
import g1_variants_runner


class Fixture(BaseHTTPRequestHandler):
    def log_message(self, *args): pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        thinking = request.get('chat_template_kwargs', {}).get('enable_thinking')
        multi = request.get('n') == 2
        # Match vLLM: greedy sampling cannot return multiple choices.
        if multi and request.get('temperature') == 0:
            body = b'{"error":{"message":"n must be 1 when using greedy sampling","code":400}}'
            self.send_response(400); self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(body))); self.end_headers(); self.wfile.write(body)
            return
        tool = bool(request.get('tools'))
        def event(index, delta, finish=None):
            return {'choices': [{'index': index, 'delta': delta, 'finish_reason': finish}]}
        events = [event(0, {'role': 'assistant'})]
        if multi: events.append(event(1, {'role': 'assistant'}))
        if thinking: events.append(event(0, {'reasoning': 'thought'}))
        if tool:
            events.append(event(0, {'tool_calls': [{'index': 0, 'function': {'name': 'get_weather', 'arguments': ''}}]}))
            events.append(event(0, {'tool_calls': [{'index': 0, 'function': {'arguments': '{"city":"test"}'}}]}))
        else:
            events.append(event(0, {'content': 'answer'}))
        if multi: events.append(event(1, {'content': 'other'}))
        events.append(event(0, {}, 'tool_calls' if tool else 'stop'))
        if multi: events.append(event(1, {}, 'stop'))
        events.append({'choices': [], 'usage': {'prompt_tokens': 4, 'completion_tokens': 8, 'total_tokens': 12}})
        chunks = [(b'data: ' + json.dumps(event).encode() + b'\n\n') for event in events] + [b'data: [DONE]\n\n']
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.send_header('Content-Length', str(sum(map(len, chunks))))
        self.send_header('Connection', 'close')
        self.end_headers()
        for chunk in chunks:
            self.wfile.write(chunk); self.wfile.flush(); time.sleep(.004)
        self.close_connection = True


@unittest.skipUnless(os.environ.get('CERT_TEST_LLM_BIN'), 'optional built CLI fixture')
class RunnerTests(unittest.TestCase):
    def test_actual_cli_six_cells(self):
        server = ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
        thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
        try:
            with tempfile.TemporaryDirectory() as tmp:
                out = Path(tmp); (out / 'evidence').mkdir()
                prompt = out / 'prompts.jsonl'; prompt.write_text('{"prompt":"hello"}\n')
                with socket.socket() as sock:
                    sock.bind(('127.0.0.1', 0)); proxy_port = sock.getsockname()[1]
                args = SimpleNamespace(candidate=Path(os.environ['CERT_TEST_LLM_BIN']), out=out,
                                       model='fixture', port=server.server_port, proxy_port=proxy_port)
                on = {'chat_template_kwargs': {'enable_thinking': True}}
                off = {'chat_template_kwargs': {'enable_thinking': False}}
                cases = [('reason', on, None, 'reasoning'), ('answer', off, None, 'answer_only'),
                         ('tool', {**off, 'tools': [g1_variants_runner.TOOL], 'tool_choice': 'required'}, None, 'tool'),
                         ('multi', {**off, 'n': 2}, 'unselected-first', 'unselected_before_selected'),
                         ('reverse', on, 'answer-first', 'answer_before_reasoning'),
                         ('same', on, 'same-event', 'same_event')]
                for label, body, mode, required in cases:
                    with self.subTest(cell=label):
                        data, trace, code = g1_variants_runner.run_cell(args, label, prompt, body, mode)
                        g1_variants_checks.evaluate(data, trace, code, mode, required)
        finally:
            server.shutdown(); server.server_close(); thread.join(timeout=10)


if __name__ == '__main__':
    unittest.main()
