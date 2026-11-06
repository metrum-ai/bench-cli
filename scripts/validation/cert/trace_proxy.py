#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Loopback-only streaming recorder for real serving responses.

Recorded response bytes can contain generated content and must be treated as
private raw evidence. This component does not synthesize server responses.
"""
import argparse
import base64
import datetime as dt
import hashlib
import http.client
import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import fault_transform
import sse_reorder

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
        record = {"request": request_id, "kind": kind, "monotonic_ns": time.monotonic_ns(),
                  "utc": dt.datetime.now(dt.timezone.utc).isoformat(), **extra}
        if data is not None:
            record["bytes_b64"] = base64.b64encode(data).decode("ascii")
        with self.lock:
            self.stream.write(json.dumps(record, separators=(",", ":")) + "\n")
            self.stream.flush()


def take_frame(buffer):
    boundaries = [(buffer.find(marker), marker) for marker in (b"\n\n", b"\r\n\r\n")]
    boundaries = [(index, marker) for index, marker in boundaries if index >= 0]
    if not boundaries:
        return None, buffer
    index, marker = min(boundaries, key=lambda item: item[0])
    end = index + len(marker)
    return buffer[:end], buffer[end:]


def request_identity(body):
    data = json.loads(body)
    messages = data.get("messages")
    if not isinstance(messages, list) or len(messages) != 1 or messages[0].get("role") != "user":
        raise ValueError("comparison requires one user message")
    text = messages[0].get("content")
    if isinstance(text, list) and all(isinstance(x, dict) and x.get("type") == "text" and isinstance(x.get("text"), str) for x in text):
        text = "".join(x["text"] for x in text)
    if not isinstance(text, str):
        raise ValueError("comparison requires text-only user prompts")
    cap = data.get("max_tokens", data.get("max_completion_tokens"))
    thinking = data.get("chat_template_kwargs", {}).get("enable_thinking")
    return {"prompt_sha256": hashlib.sha256(text.encode("utf-8")).hexdigest(), "max_tokens": cap,
            "model": data.get("model"), "temperature": data.get("temperature"), "seed": data.get("seed"),
            "thinking": thinking, "streaming": data.get("stream", False)}


def image_identity(body):
    data = json.loads(body)
    images = []
    for message in data.get('messages', []):
        content = message.get('content')
        if not isinstance(content, list):
            continue
        for part in content:
            if isinstance(part, dict) and part.get('type') == 'image_url':
                url = (part.get('image_url') or {}).get('url', '')
                header, separator, encoded = url.partition(',')
                if not separator or not header.startswith('data:image/') or not header.endswith(';base64'):
                    raise ValueError('image must be an embedded data URL')
                raw = base64.b64decode(encoded, validate=True)
                if not raw:
                    raise ValueError('empty image')
                images.append(hashlib.sha256(raw).hexdigest())
    if not images:
        raise ValueError('no image sent')
    return {'image_sha256': images, 'model': data.get('model'),
            'thinking': data.get('chat_template_kwargs', {}).get('enable_thinking')}


def wait_for_upstream(port, recorder, request_id, retry_seconds):
    """Gate new POSTs on real-server readiness; never replay a submitted POST."""
    deadline = time.monotonic() + retry_seconds
    while True:
        probe = http.client.HTTPConnection("localhost", port, timeout=2)
        try:
            probe.request("GET", "/health")
            response = probe.getresponse()
            response.read()
            if response.status == 200:
                recorder.event(request_id, "upstream_ready")
                return
        except (OSError, http.client.HTTPException):
            pass
        finally:
            probe.close()
        if time.monotonic() >= deadline:
            raise TimeoutError("real server did not recover before readiness deadline")
        recorder.event(request_id, "upstream_wait")
        time.sleep(.25)


def make_handler(upstream_port, recorder, fault=None, fault_request=None, request_caps=None, retry_seconds=0, record_identity=False, delay_request=None, delay_seconds=0, record_image_identity=False, reorder=None):
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
            if request_caps:
                try:
                    data = json.loads(body)
                    if not isinstance(data, dict) or not isinstance(data.get("max_tokens"), int) or isinstance(data["max_tokens"], bool):
                        raise ValueError("request lacks an integer max_tokens")
                    original_hash = hashlib.sha256(body).hexdigest()
                    original_cap = data["max_tokens"]
                    cap = request_caps[(request_id - 1) % len(request_caps)]
                    data["max_tokens"] = cap
                    body = json.dumps(data, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
                    recorder.event(request_id, "request_transform", original_cap=original_cap, forwarded_cap=cap,
                                   original_sha256=original_hash, forwarded_sha256=hashlib.sha256(body).hexdigest())
                except (ValueError, UnicodeError):
                    recorder.event(request_id, "error", error="InvalidRequestForCapTransform")
                    self.send_error(400)
                    return
            if record_identity:
                try:
                    recorder.event(request_id, "request_identity", **request_identity(body))
                except (ValueError, TypeError, AttributeError, json.JSONDecodeError):
                    recorder.event(request_id, "error", error="InvalidComparisonRequest")
                    self.send_error(400)
                    return
            if record_image_identity:
                try:
                    recorder.event(request_id, 'image_identity', **image_identity(body))
                except (ValueError, TypeError, AttributeError, json.JSONDecodeError):
                    recorder.event(request_id, 'error', error='InvalidImageRequest')
                    self.send_error(400)
                    return
            upstream = http.client.HTTPConnection("localhost", upstream_port, timeout=300)
            try:
                headers = {key: value for key, value in self.headers.items() if key.lower() not in HOP_HEADERS}
                headers["Content-Length"] = str(len(body))
                recorder.event(request_id, "send")
                if retry_seconds:
                    wait_for_upstream(upstream_port, recorder, request_id, retry_seconds)
                deadline = time.monotonic() + retry_seconds
                while True:
                    try:
                        upstream.request("POST", self.path, body=body, headers=headers)
                        break
                    except ConnectionRefusedError:
                        upstream.close()
                        if time.monotonic() >= deadline:
                            raise
                        recorder.event(request_id, "upstream_wait")
                        time.sleep(0.25)
                        upstream = http.client.HTTPConnection("localhost", upstream_port, timeout=300)
                response = upstream.getresponse()
                recorder.event(request_id, "headers", status=response.status)
                self.send_response(response.status)
                for key, value in response.getheaders():
                    if key.lower() not in HOP_HEADERS:
                        self.send_header(key, value)
                self.send_header("Connection", "close")
                self.end_headers()
                self.close_connection = True
                selected = fault is not None and (fault_request is None or fault_request == request_id)
                buffer = b""
                transformed = 0
                delayed = False

                def forward(chunk):
                    recorder.event(request_id, "forwarded_chunk", chunk)
                    self.wfile.write(chunk)
                    self.wfile.flush()

                while True:
                    chunk = response.read1(65536)
                    if not chunk:
                        break
                    recorder.event(request_id, "chunk", chunk)
                    if delay_request == request_id and not delayed:
                        recorder.event(request_id, "delay", seconds=delay_seconds)
                        time.sleep(delay_seconds)
                        delayed = True
                    if reorder:
                        buffer += chunk
                        if len(buffer) > 16 * 1024 * 1024:
                            raise ValueError('source stream exceeds reorder limit')
                        continue
                    if not selected:
                        forward(chunk)
                        continue
                    buffer += chunk
                    if len(buffer) > 16 * 1024 * 1024:
                        raise ValueError("SSE frame exceeds recorder limit")
                    while True:
                        frame, buffer = take_frame(buffer)
                        if frame is None:
                            break
                        # Corrupt only one real frame per selected request.
                        if fault == "corrupt" and transformed:
                            result, changed = frame, False
                        else:
                            result, changed = fault_transform.transform(frame, fault)
                        if changed:
                            transformed += 1
                            recorder.event(request_id, "transformation", mode=fault)
                        forward(result)
                if reorder:
                    frames = []
                    while buffer:
                        frame, buffer = take_frame(buffer)
                        if frame is None:
                            raise ValueError('incomplete source stream')
                        frames.append(frame)
                    output = sse_reorder.transform(frames, reorder)
                    recorder.event(request_id, 'transformation', mode=reorder, original_frames=len(frames), forwarded_frames=len(output))
                    transformed += 1
                    for frame in output:
                        forward(frame)
                elif buffer:
                    forward(buffer)
                recorder.event(request_id, "complete", fault=reorder or (fault if selected else None), transformations=transformed)
            except (OSError, ValueError, http.client.HTTPException) as exc:
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
    parser.add_argument("--fault", choices=fault_transform.MODES)
    parser.add_argument('--reorder', choices=sse_reorder.MODES, help='transform complete real SSE streams for clock-boundary tests')
    parser.add_argument("--fault-request", type=int, help="apply fault only to this one-based request index")
    parser.add_argument("--request-caps", help="cycle these comma-separated request max_tokens values")
    parser.add_argument("--retry-seconds", type=int, default=0, help="wait for real-server readiness before sending during restart tests")
    parser.add_argument("--record-request-identity", action="store_true", help="record normalized prompt hashes and comparison settings")
    parser.add_argument("--record-image-identity", action="store_true", help="record hashes of embedded request images without their bytes")
    parser.add_argument("--delay-request", type=int, help="delay first real response chunk for this one-based request")
    parser.add_argument("--delay-seconds", type=float, default=0)
    args = parser.parse_args()
    if not 1024 <= args.listen_port <= 65535 or not 1024 <= args.upstream_port <= 65535 or args.listen_port == args.upstream_port:
        parser.error("ports must be distinct and in 1024..65535")
    if args.reorder and (args.fault or args.delay_request or args.request_caps or args.retry_seconds):
        parser.error('--reorder cannot be combined with other proxy transformations')
    if args.fault_request is not None and (args.fault is None or args.fault_request < 1):
        parser.error("--fault-request requires a fault and a positive request index")
    if args.delay_request is not None and (args.delay_request < 1 or not 0 < args.delay_seconds <= 30):
        parser.error("--delay-request requires a positive index and --delay-seconds in (0,30]")
    if args.delay_request is None and args.delay_seconds != 0:
        parser.error("--delay-seconds requires --delay-request")
    if not 0 <= args.retry_seconds <= 300:
        parser.error("--retry-seconds must be in 0..300")
    caps = None
    if args.request_caps:
        try:
            caps = [int(value) for value in args.request_caps.split(",")]
            if not caps or any(value < 1 or value > 65536 for value in caps):
                raise ValueError("invalid cap")
        except ValueError:
            parser.error("--request-caps must be comma-separated integers in 1..65536")
    recorder = Recorder(args.out)
    server = ThreadingHTTPServer(("localhost", args.listen_port), make_handler(args.upstream_port, recorder, args.fault, args.fault_request, caps, args.retry_seconds, args.record_request_identity, args.delay_request, args.delay_seconds, args.record_image_identity, args.reorder))
    try:
        server.serve_forever()
    finally:
        server.server_close()
        recorder.stream.close()


if __name__ == "__main__":
    main()
