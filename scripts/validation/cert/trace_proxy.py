#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Loopback-only streaming recorder for real serving responses.

Recorded response bytes can contain generated content and must be treated as
private raw evidence. This component does not synthesize server responses.
"""
import argparse
import base64
import http.client
import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

HOP_HEADERS = {"connection", "keep-alive", "proxy-authenticate", "proxy-authorization", "te", "trailer", "transfer-encoding", "upgrade", "host", "content-length"}


class Recorder:
    def __init__(self, path):
        self.stream = Path(path).open("x", encoding="utf-8")
        self.lock = threading.Lock()
        self.next_id = 0

    def allocate(self):
        with self.lock:
            self.next_id += 1
            return self.next_id

    def event(self, request_id, kind, data=None, **extra):
        record = {"request": request_id, "kind": kind, "monotonic_ns": time.monotonic_ns(), **extra}
        if data is not None:
            record["bytes_b64"] = base64.b64encode(data).decode("ascii")
        with self.lock:
            self.stream.write(json.dumps(record, separators=(",", ":")) + "\n")
            self.stream.flush()


def make_handler(upstream_port, recorder):
    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, fmt, *args):
            # Avoid logging headers, prompts or arbitrary request data.
            return

        def do_POST(self):
            request_id = recorder.allocate()
            if not self.path.startswith("/v1/") or self.headers.get("Transfer-Encoding"):
                self.send_error(400)
                return
            try:
                size = int(self.headers.get("Content-Length", "-1"))
            except ValueError:
                size = -1
            if size < 0 or size > 16 * 1024 * 1024:
                self.send_error(413)
                return
            body = self.rfile.read(size)
            if len(body) != size:
                self.send_error(400)
                return
            upstream = http.client.HTTPConnection("localhost", upstream_port, timeout=300)
            try:
                headers = {key: value for key, value in self.headers.items() if key.lower() not in HOP_HEADERS}
                headers["Content-Length"] = str(len(body))
                recorder.event(request_id, "send")
                upstream.request("POST", self.path, body=body, headers=headers)
                response = upstream.getresponse()
                recorder.event(request_id, "headers", status=response.status)
                self.send_response(response.status)
                for key, value in response.getheaders():
                    if key.lower() not in HOP_HEADERS:
                        self.send_header(key, value)
                self.send_header("Connection", "close")
                self.end_headers()
                self.close_connection = True
                while True:
                    chunk = response.read1(65536)
                    if not chunk:
                        break
                    recorder.event(request_id, "chunk", chunk)
                    self.wfile.write(chunk)
                    self.wfile.flush()
                recorder.event(request_id, "complete")
            except (OSError, http.client.HTTPException) as exc:
                recorder.event(request_id, "error", error=type(exc).__name__)
                self.close_connection = True
            finally:
                upstream.close()

    return Handler


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--listen-port", type=int, required=True)
    parser.add_argument("--upstream-port", type=int, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if not 1024 <= args.listen_port <= 65535 or not 1024 <= args.upstream_port <= 65535 or args.listen_port == args.upstream_port:
        parser.error("ports must be distinct and in 1024..65535")
    recorder = Recorder(args.out)
    server = ThreadingHTTPServer(("localhost", args.listen_port), make_handler(args.upstream_port, recorder))
    try:
        server.serve_forever()
    finally:
        server.server_close()
        recorder.stream.close()


if __name__ == "__main__":
    main()
