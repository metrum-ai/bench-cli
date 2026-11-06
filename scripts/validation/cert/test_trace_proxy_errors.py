# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Error responses must not be treated as reorderable SSE."""
import base64
import http.client
import json
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import trace_proxy

ERROR_BODY = b'{"error":{"message":"n must be 1 when using greedy sampling","code":400}}'


class ErrorServer(BaseHTTPRequestHandler):
    def log_message(self, *args): pass

    def do_POST(self):
        self.rfile.read(int(self.headers['Content-Length']))
        self.send_response(400)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(ERROR_BODY)))
        self.end_headers()
        self.wfile.write(ERROR_BODY)


class ReorderErrorTests(unittest.TestCase):
    def test_non_sse_error_is_forwarded_unchanged(self):
        upstream = ThreadingHTTPServer(('127.0.0.1', 0), ErrorServer)
        thread = threading.Thread(target=upstream.serve_forever, daemon=True)
        thread.start()
        try:
            with tempfile.TemporaryDirectory() as tmp:
                trace = Path(tmp) / 'trace.jsonl'
                recorder = trace_proxy.Recorder(trace)
                handler = trace_proxy.make_handler(upstream.server_port, recorder, reorder='unselected-first')
                proxy = ThreadingHTTPServer(('127.0.0.1', 0), handler)
                proxy_thread = threading.Thread(target=proxy.serve_forever, daemon=True)
                proxy_thread.start()
                try:
                    connection = http.client.HTTPConnection('127.0.0.1', proxy.server_port, timeout=5)
                    connection.request('POST', '/v1/chat/completions', body=b'{}', headers={'Content-Type': 'application/json'})
                    response = connection.getresponse()
                    self.assertEqual(response.status, 400)
                    self.assertEqual(response.read(), ERROR_BODY)
                    connection.close()
                finally:
                    proxy.shutdown(); proxy.server_close(); proxy_thread.join(timeout=10)
                    recorder.stream.close()
                events = [json.loads(line) for line in trace.read_text().splitlines()]
                self.assertFalse(any(x['kind'] in ('error', 'transformation') for x in events))
                self.assertEqual(b''.join(base64.b64decode(x['bytes_b64']) for x in events if x['kind'] == 'forwarded_chunk'), ERROR_BODY)
                self.assertEqual(sum(x['kind'] == 'complete' for x in events), 1)
        finally:
            upstream.shutdown(); upstream.server_close(); thread.join(timeout=10)


if __name__ == '__main__':
    unittest.main()
