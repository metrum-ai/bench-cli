#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Metrum AI Bench CLI parity harness: paced OpenAI-compatible SSE mock.

Built by Metrum AI for the #204 count harness. A count is only meaningful when
the client sees a real stream, so every response is paced:

  first token at  prefill_ms + per_prompt_token_ms * prompt_tokens + itl_ms
  token i at      first token + (i - 1) * itl_ms

The output length is 60-100% of the request's max_tokens (or
max_completion_tokens), chosen by a seeded RNG keyed on the prompt text, so
both tools see the same length for the same prompt. Usage arrives in the final
chunk. With --reasoning the first share of tokens goes out as
delta.reasoning_content. GET /metrics serves a vLLM-style page with counters,
gauges, and exactly one histogram (vllm:e2e_request_latency_seconds).

Each SSE event is written and flushed as its own HTTP chunk with TCP_NODELAY.
Never swap this for a mock that writes the whole stream at once: TTFT then
equals E2E and count_points.py refuses the run.

Usage: mock_server.py [--host 127.0.0.1] [--port 8000] [--model parity-mock]
           [--prefill-ms 40] [--per-prompt-token-ms 0.05] [--itl-ms 8]
           [--min-frac 0.6] [--max-frac 1.0] [--seed 0]
           [--reasoning] [--reasoning-frac 0.3]
Standard library only.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import random
import socket
import threading
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

# Upper bounds (seconds) for the one histogram on /metrics.
LATENCY_BUCKETS = (0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0)
# Each " word" is one GPT-2 token, so server usage and a client tokenizer agree.
WORDS = ("the", "of", "and", "to", "in", "is", "it", "that", "for", "on", "with", "as")


class Stats:
    """Process-wide counters behind GET /metrics (Metrum AI parity mock)."""

    def __init__(self) -> None:
        self.lock = threading.Lock()
        self.running = 0
        self.requests = 0
        self.prompt_tokens = 0
        self.generation_tokens = 0
        self.bucket_counts = [0] * len(LATENCY_BUCKETS)
        self.latency_sum = 0.0
        self.latency_count = 0

    def start(self) -> None:
        with self.lock:
            self.running += 1

    def finish(self, prompt_tokens: int, completion_tokens: int, latency_s: float) -> None:
        with self.lock:
            self.running -= 1
            self.requests += 1
            self.prompt_tokens += prompt_tokens
            self.generation_tokens += completion_tokens
            self.latency_sum += latency_s
            self.latency_count += 1
            for i, upper in enumerate(LATENCY_BUCKETS):
                if latency_s <= upper:
                    self.bucket_counts[i] += 1

    def page(self, model: str) -> str:
        lbl = f'model_name="{model}"'
        with self.lock:
            lines = [
                "# HELP vllm:num_requests_running Number of requests in model execution batches.",
                "# TYPE vllm:num_requests_running gauge",
                f"vllm:num_requests_running{{{lbl}}} {self.running}",
                "# HELP vllm:num_requests_waiting Number of requests waiting to be processed.",
                "# TYPE vllm:num_requests_waiting gauge",
                f"vllm:num_requests_waiting{{{lbl}}} 0",
                "# HELP vllm:kv_cache_usage_perc KV-cache usage. 1 means 100 percent usage.",
                "# TYPE vllm:kv_cache_usage_perc gauge",
                f"vllm:kv_cache_usage_perc{{{lbl}}} {min(1.0, 0.05 * self.running):.4f}",
                "# HELP vllm:prompt_tokens_total Number of prefill tokens processed.",
                "# TYPE vllm:prompt_tokens_total counter",
                f"vllm:prompt_tokens_total{{{lbl}}} {self.prompt_tokens}",
                "# HELP vllm:generation_tokens_total Number of generation tokens processed.",
                "# TYPE vllm:generation_tokens_total counter",
                f"vllm:generation_tokens_total{{{lbl}}} {self.generation_tokens}",
                "# HELP vllm:request_success_total Count of successfully processed requests.",
                "# TYPE vllm:request_success_total counter",
                f"vllm:request_success_total{{{lbl}}} {self.requests}",
                "# HELP vllm:e2e_request_latency_seconds Histogram of end to end request latency in seconds.",
                "# TYPE vllm:e2e_request_latency_seconds histogram",
            ]
            # bucket_counts are cumulative already: finish() bumps every le >= latency.
            for upper, count in zip(LATENCY_BUCKETS, self.bucket_counts):
                lines.append(f'vllm:e2e_request_latency_seconds_bucket{{{lbl},le="{upper}"}} {count}')
            lines.append(f'vllm:e2e_request_latency_seconds_bucket{{{lbl},le="+Inf"}} {self.latency_count}')
            lines.append(f"vllm:e2e_request_latency_seconds_sum{{{lbl}}} {self.latency_sum:.6f}")
            lines.append(f"vllm:e2e_request_latency_seconds_count{{{lbl}}} {self.latency_count}")
        return "\n".join(lines) + "\n"


def prompt_text(body: dict) -> str:
    """Concatenate chat message text (or the completions prompt)."""
    if "messages" in body:
        parts = []
        for msg in body.get("messages") or []:
            content = msg.get("content")
            if isinstance(content, str):
                parts.append(content)
            elif isinstance(content, list):
                parts.extend(p.get("text", "") for p in content if isinstance(p, dict))
        return "\n".join(parts)
    prompt = body.get("prompt", "")
    return prompt if isinstance(prompt, str) else json.dumps(prompt)


def count_prompt_tokens(text: str) -> int:
    """Whitespace word count; deterministic, identical for both tools, and equal
    to the GPT-2 count for the harness prompts (single-token words)."""
    return max(1, len(text.split()))


def make_handler(args: argparse.Namespace, stats: Stats):
    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def setup(self) -> None:
            super().setup()
            # Flush every SSE event onto the wire immediately.
            self.connection.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)

        def log_message(self, fmt: str, *a) -> None:  # quiet by default
            if args.verbose:
                super().log_message(fmt, *a)

        def send_json(self, code: int, obj: dict) -> None:
            data = json.dumps(obj).encode()
            self.send_response(code)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self) -> None:
            path = self.path.split("?", 1)[0]
            if path == "/metrics":
                data = stats.page(args.model).encode()
                self.send_response(200)
                self.send_header("Content-Type", "text/plain; version=0.0.4")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)
            elif path in ("/v1/models", "/models"):
                self.send_json(200, {"object": "list", "data": [
                    {"id": args.model, "object": "model", "owned_by": "metrum-ai-parity"}]})
            elif path in ("/health", "/v1/health"):
                self.send_json(200, {"status": "ok"})
            else:
                self.send_json(404, {"error": {"message": f"no route {path}"}})

        def do_POST(self) -> None:
            path = self.path.split("?", 1)[0]
            length = int(self.headers.get("Content-Length") or 0)
            try:
                body = json.loads(self.rfile.read(length) or b"{}")
            except json.JSONDecodeError:
                self.send_json(400, {"error": {"message": "invalid JSON"}})
                return
            if path not in ("/v1/chat/completions", "/v1/completions"):
                self.send_json(404, {"error": {"message": f"no route {path}"}})
                return
            self.serve_completion(body, chat=path.endswith("chat/completions"))

        def serve_completion(self, body: dict, chat: bool) -> None:
            t0 = time.monotonic()
            text = prompt_text(body)
            prompt_tokens = count_prompt_tokens(text)
            max_tokens = int(body.get("max_completion_tokens") or body.get("max_tokens") or 128)
            key = hashlib.sha256(f"{args.seed}|{max_tokens}|{text}".encode()).digest()
            rng = random.Random(int.from_bytes(key[:8], "big"))
            n_out = max(1, round(max_tokens * rng.uniform(args.min_frac, args.max_frac)))
            n_reason = int(n_out * args.reasoning_frac) if (args.reasoning and chat) else 0
            first_at = (args.prefill_ms + args.per_prompt_token_ms * prompt_tokens + args.itl_ms) / 1000.0
            itl = args.itl_ms / 1000.0
            rid = f"{'chatcmpl' if chat else 'cmpl'}-{uuid.uuid4().hex[:24]}"
            created = int(time.time())
            obj = "chat.completion.chunk" if chat else "text_completion"
            finish = "length" if n_out >= max_tokens else "stop"
            usage = {"prompt_tokens": prompt_tokens, "completion_tokens": n_out,
                     "total_tokens": prompt_tokens + n_out}
            if n_reason:
                usage["completion_tokens_details"] = {"reasoning_tokens": n_reason}
            stats.start()
            try:
                if body.get("stream"):
                    self.stream(rid, created, obj, chat, n_out, n_reason, finish, usage, t0, first_at, itl)
                else:
                    time.sleep(max(0.0, t0 + first_at + (n_out - 1) * itl - time.monotonic()))
                    self.whole(rid, created, chat, n_out, n_reason, finish, usage)
            finally:
                stats.finish(prompt_tokens, n_out, time.monotonic() - t0)

        def chunk(self, payload: str) -> None:
            data = payload.encode()
            self.wfile.write(f"{len(data):x}\r\n".encode() + data + b"\r\n")
            self.wfile.flush()

        def stream(self, rid, created, obj, chat, n_out, n_reason, finish, usage, t0, first_at, itl) -> None:
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Cache-Control", "no-cache")
            self.send_header("Transfer-Encoding", "chunked")
            self.end_headers()
            base = {"id": rid, "object": obj, "created": created, "model": args.model}
            for i in range(n_out):
                # Absolute deadlines so sleep jitter does not accumulate.
                delay = t0 + first_at + i * itl - time.monotonic()
                if delay > 0:
                    time.sleep(delay)
                word = " " + WORDS[i % len(WORDS)]
                last = i == n_out - 1
                if chat:
                    delta = {"reasoning_content": word} if i < n_reason else {"content": word}
                    if i == 0:
                        delta["role"] = "assistant"
                    choice = {"index": 0, "delta": delta, "finish_reason": finish if last else None}
                else:
                    choice = {"index": 0, "text": word, "finish_reason": finish if last else None}
                self.chunk("data: " + json.dumps({**base, "choices": [choice]}) + "\n\n")
            # Usage in the final chunk (OpenAI include_usage shape), sent always.
            self.chunk("data: " + json.dumps({**base, "choices": [], "usage": usage}) + "\n\n")
            self.chunk("data: [DONE]\n\n")
            self.wfile.write(b"0\r\n\r\n")
            self.wfile.flush()

        def whole(self, rid, created, chat, n_out, n_reason, finish, usage) -> None:
            words = [WORDS[i % len(WORDS)] for i in range(n_out)]
            if chat:
                message = {"role": "assistant", "content": " ".join(words[n_reason:])}
                if n_reason:
                    message["reasoning_content"] = " ".join(words[:n_reason])
                choice = {"index": 0, "message": message, "finish_reason": finish}
                obj = "chat.completion"
            else:
                choice = {"index": 0, "text": " ".join(words), "finish_reason": finish}
                obj = "text_completion"
            self.send_json(200, {"id": rid, "object": obj, "created": created, "model": args.model,
                                 "choices": [choice], "usage": usage})

    return Handler


def main() -> int:
    ap = argparse.ArgumentParser(description="Metrum AI paced SSE mock for the parity count harness")
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=8000)
    ap.add_argument("--model", default="parity-mock")
    ap.add_argument("--prefill-ms", type=float, default=40.0, help="fixed delay before the first token")
    ap.add_argument("--per-prompt-token-ms", type=float, default=0.05, help="extra prefill per prompt token")
    ap.add_argument("--itl-ms", type=float, default=8.0, help="fixed gap between output tokens")
    ap.add_argument("--min-frac", type=float, default=0.6, help="lowest output share of max_tokens")
    ap.add_argument("--max-frac", type=float, default=1.0, help="highest output share of max_tokens")
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--reasoning", action="store_true", help="emit delta.reasoning_content first")
    ap.add_argument("--reasoning-frac", type=float, default=0.3, help="share of output tokens that reason")
    ap.add_argument("--verbose", action="store_true")
    args = ap.parse_args()
    if args.itl_ms <= 0:
        ap.error("--itl-ms must be > 0: a zero gap is a single-chunk mock and invalidates the count")
    if not 0 < args.min_frac <= args.max_frac <= 1.0:
        ap.error("need 0 < --min-frac <= --max-frac <= 1")
    server = ThreadingHTTPServer((args.host, args.port), make_handler(args, Stats()))
    server.daemon_threads = True
    print(f"parity mock listening on http://{args.host}:{args.port} model={args.model} "
          f"reasoning={args.reasoning}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
