#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Capacity gate in front of metrum-ai-bench-cli-mock-server (record.sh only).

The mock serves every request after a fixed --latency-ms, so a concurrency
sweep against it never bends. This proxy lets at most CAPACITY requests reach
the mock at once and queues the rest, the way a saturated serving engine does:
p95 stays flat up to CAPACITY, then grows with the queue. Fixture tooling, not
a benchmark component.

Usage: capacity_proxy.py LISTEN_PORT UPSTREAM_URL CAPACITY
"""

from __future__ import annotations

import http.client
import sys
import threading
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def main(argv):
    port, upstream, capacity = int(argv[1]), urllib.parse.urlsplit(argv[2]), int(argv[3])
    gate = threading.BoundedSemaphore(capacity)

    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def forward(self, method):
            length = int(self.headers.get("Content-Length") or 0)
            body = self.rfile.read(length) if length else None
            headers = {"Content-Type": self.headers.get("Content-Type", "application/json")}
            with gate:
                conn = http.client.HTTPConnection(upstream.hostname, upstream.port, timeout=60)
                try:
                    conn.request(method, self.path, body=body, headers=headers)
                    resp = conn.getresponse()
                    data = resp.read()
                    ctype = resp.getheader("Content-Type")
                    status = resp.status
                finally:
                    conn.close()
            self.send_response(status)
            if ctype:
                self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):
            self.forward("GET")

        def do_POST(self):
            self.forward("POST")

        def log_message(self, *_):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    server.daemon_threads = True
    server.serve_forever()


if __name__ == "__main__":
    sys.exit(main(sys.argv))
