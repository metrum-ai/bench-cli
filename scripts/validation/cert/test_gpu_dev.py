# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""CPU-only regression tests for GPU lane infrastructure. No GPU is used."""
import base64
import hashlib
import http.client
import http.server
import json
import os
import socket
import sys
import time
import tempfile
import threading
import unittest
from pathlib import Path
from unittest import mock

import d_default_checks
import g1_clock_checks
import g6_checks
import l1_runner
import manifest_tool
import r_checks
import result_writer
import schema_checks
import trace_proxy


class ManifestTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.candidate = self.root / "metrum-ai-bench-cli-llm"
        self.baseline = self.root / "baseline"
        self.helper = self.root / "metrum-ai-bench-cli-prompts"
        self.prompt = self.root / "prompts"
        self.candidate.write_bytes(b"candidate")
        self.baseline.write_bytes(b"baseline")
        self.helper.write_bytes(b"helper")
        self.prompt.write_bytes(b"prompt")
        self.sut = self.root / "sut.json"
        self.sut.write_text(json.dumps({"model": {"id": "Qwen/Qwen3-8B", "revision": "c" * 40}}), encoding="utf-8")
        os.chmod(self.candidate, 0o755)
        os.chmod(self.baseline, 0o755)
        os.chmod(self.helper, 0o755)
        digest = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
        self.manifest = {"candidate_sha": "a" * 40,
                         "candidate_binary_sha256": digest(self.candidate),
                         "baseline_binary_sha256": digest(self.baseline),
                         "lanes": {"L1": {"image": "example/image@sha256:" + "b" * 64,
                                           "model": "Qwen/Qwen3-8B", "revision": "c" * 40,
                                           "port": 8001, "server_flags": ["--max-model-len", "4096"],
                                           "sut": {"path": str(self.sut), "sha256": digest(self.sut)},
                                           "prompt_binary": {"path": str(self.helper), "sha256": digest(self.helper)},
                                           "owner": "tester", "evidence_link": "https://example.test/run",
                                           "prompt_sets": {"default": {"path": str(self.prompt),
                                                                       "sha256": digest(self.prompt)}}}}}
        self.path = self.root / "manifest.json"

    def load(self, gpu="0"):
        self.path.write_text(json.dumps(self.manifest), encoding="utf-8")
        return manifest_tool.load_manifest(self.path, "L1", self.candidate, self.baseline, gpu)

    def test_valid_pins(self):
        _, config = self.load()
        self.assertEqual(8001, config["port"])

    def test_reasoning_lane_requires_proxy_and_pinned_strategic(self):
        config = self.manifest["lanes"]["L1"].copy()
        config.update({"proxy_port": 8111, "server_flags": ["--reasoning-parser", "qwen3", "--no-enable-prefix-caching"]})
        self.manifest["lanes"]["R"] = config.copy()
        self.manifest["lanes"]["G1"] = config.copy()
        strategic = self.root / "metrum-ai-bench-cli-strategic"
        strategic.write_text("binary")
        strategic.chmod(0o755)
        self.manifest["lanes"]["G6"] = {**config, "strategic_binary": {"path": str(strategic), "sha256": manifest_tool.sha256_file(strategic)}}
        self.path.write_text(json.dumps(self.manifest))
        manifest_tool.load_manifest(self.path, "R", self.candidate, self.baseline, "0")
        manifest_tool.load_manifest(self.path, "G1", self.candidate, self.baseline, "0")
        manifest_tool.load_manifest(self.path, "G6", self.candidate, self.baseline, "0")
        self.manifest["lanes"]["G6"]["strategic_binary"]["sha256"] = "0" * 64
        self.path.write_text(json.dumps(self.manifest))
        with self.assertRaises(ValueError):
            manifest_tool.load_manifest(self.path, "G6", self.candidate, self.baseline, "0")

    def test_g7_requires_both_frozen_workloads_and_proxy(self):
        config = self.manifest["lanes"]["L1"].copy()
        default = config["prompt_sets"]["default"]
        config.update({"proxy_port": 8111, "server_flags": ["--no-enable-prefix-caching"],
                       "prompt_sets": {"W1": default, "W2": default}})
        self.manifest["lanes"]["G7"] = config
        self.path.write_text(json.dumps(self.manifest))
        manifest_tool.load_manifest(self.path, "G7", self.candidate, self.baseline, "0")
        del config["prompt_sets"]["W2"]
        self.path.write_text(json.dumps(self.manifest))
        with self.assertRaises(ValueError):
            manifest_tool.load_manifest(self.path, "G7", self.candidate, self.baseline, "0")

    def test_rejects_invalid_gpu_image_flags_and_hash(self):
        with self.assertRaises(ValueError):
            self.load("0,1")
        config = self.manifest["lanes"]["L1"]
        for key, value in (("image", "example/image:latest"), ("port", 80),
                           ("server_flags", ["--port=9999"]), ("revision", "main")):
            original = config[key]
            config[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.load()
            config[key] = original
        self.prompt.write_bytes(b"changed")
        with self.assertRaises(ValueError):
            self.load()


class G6EvaluatorTests(unittest.TestCase):
    def fixture(self):
        rows = [{"schema_version": "metrum-ai-bench-cli.request.v5", "phase": "measure", "error": None,
                 "usage_prompt_tokens_present": True, "usage_completion_tokens_present": True,
                 "prompt_tokens": 3, "completion_tokens": 2} for _ in range(64)]
        rows.append({"schema_version": "metrum-ai-bench-cli.summary.v5", "attempted": 64, "successes": 64,
                     "prompt_tokens_total": 192, "completion_tokens_total": 128, "window_seconds": 10,
                     "input_tokens_per_second": 19.2, "completion_tokens_per_second": 12.8,
                     "total_tokens_per_second": 32.0})
        frame = (b'data: {"choices":[{"delta":{"reasoning_content":"x"}}]}\n\n'
                 b'data: {"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":2}}\n\ndata: [DONE]\n\n')
        trace = []
        for i in range(64):
            trace.extend([{"request": i, "kind": "chunk", "bytes_b64": base64.b64encode(frame[:15]).decode()},
                          {"request": i, "kind": "chunk", "bytes_b64": base64.b64encode(frame[15:]).decode()},
                          {"request": i, "kind": "complete"}])
        return "\n".join(map(json.dumps, rows)), "\n".join(map(json.dumps, trace))

    def test_counter_label_is_bound_to_served_model_and_revision(self):
        model, revision = "Qwen/Qwen3-8B", "c" * 40
        alias = f"/root/.cache/huggingface/hub/models--Qwen--Qwen3-8B/snapshots/{revision}"
        def metrics(label):
            return f'vllm:prompt_tokens_total{{model_name="{label}"}} 1\nvllm:generation_tokens_total{{model_name="{label}"}} 2\n'
        inventory = {"data": [{"id": model}]}
        self.assertEqual((alias, "pinned-cache-snapshot"), g6_checks.metrics_model_label(metrics(alias), metrics(alias), model, revision, inventory, True))
        self.assertEqual((model, "model-id"), g6_checks.metrics_model_label(metrics(model), metrics(model), model, revision, inventory, False))
        for before, after, inventory_value, cache in ((metrics(alias), metrics(alias), inventory, False),
                                                     (metrics(alias), metrics(model), inventory, True),
                                                     (metrics(alias) + metrics(model), metrics(alias), inventory, True),
                                                     (metrics(alias), metrics(alias), {"data": []}, True)):
            with self.assertRaises(ValueError):
                g6_checks.metrics_model_label(before, after, model, revision, inventory_value, cache)

    def test_real_usage_oracle_requires_full_coverage_and_consistent_rate(self):
        data, trace = self.fixture()
        before = 'vllm:prompt_tokens_total{model_name="model"} 0\nvllm:generation_tokens_total{model_name="model"} 0\n'
        after = 'vllm:prompt_tokens_total{model_name="model"} 192\nvllm:generation_tokens_total{model_name="model"} 128\n'
        result = g6_checks.evaluate(data, trace, before, after, "model")
        self.assertEqual(64, result["requests"])
        with self.assertRaises(ValueError):
            g6_checks.evaluate(data.replace('"total_tokens_per_second": 32.0', '"total_tokens_per_second": 40.0'), trace, before, after, "model")
        with self.assertRaises(ValueError):
            g6_checks.evaluate(data, trace, before, before, "model")


class ReasoningEvaluatorTests(unittest.TestCase):
    def fixture(self, strict=False, answer=False):
        data, trace = G6EvaluatorTests().fixture()
        rows = [json.loads(line) for line in data.splitlines()]
        for row in rows[:-1]:
            row.update({"outcome": "answer" if answer else "reasoning_truncated", "answer_completed": answer,
                        "ttft_s": 0.1, "ttft_visible_answer_s": 0.2 if answer else None,
                        "error": {"kind": "reasoning_truncated"} if strict and not answer else None})
        rows[-1]["ttft_coverage"] = {"reasoning_truncated": 0 if answer else 64}
        rows[-1]["errors"] = 64 if strict and not answer else 0
        return "\n".join(map(json.dumps, rows)), trace

    def test_strict_and_default_truncation_exits(self):
        data, trace = self.fixture()
        self.assertEqual(64, r_checks.truncation_cell(data, trace, 64, False, 0)["truncated"])
        with self.assertRaises(ValueError):
            r_checks.truncation_cell(data, trace, 64, True, 0)
        data, trace = self.fixture(strict=True)
        self.assertEqual(64, r_checks.truncation_cell(data, trace, 64, True, 1)["truncated"])
        with self.assertRaises(ValueError):
            r_checks.truncation_cell(data, trace, 64, False, 0)
        self.assertEqual(64, r_checks.safety_cell(data, trace, 64, 1)["errors"])
        with self.assertRaises(ValueError):
            r_checks.safety_cell(data, trace, 64, 0)

    def test_adequate_clocks_and_rates(self):
        data, trace = self.fixture(answer=True)
        self.assertEqual(64, r_checks.adequate_cell(data, trace, 64, 0)["answered"])
        with self.assertRaises(ValueError):
            r_checks.adequate_cell(data.replace('"ttft_visible_answer_s": 0.2', '"ttft_visible_answer_s": 0.01'), trace, 64, 0)


class ClockTraceTests(unittest.TestCase):
    def test_serial_real_byte_timing_join(self):
        frame1 = b'data: {"choices":[{"index":0,"delta":{"reasoning_content":"r"}}]}\n\n'
        frame2 = b'data: {"choices":[{"index":0,"delta":{"content":"answer"}}]}\n\ndata: [DONE]\n\n'
        events = [{"request": 1, "kind": "send", "monotonic_ns": 1000000000},
                  {"request": 1, "kind": "chunk", "monotonic_ns": 1100000000, "bytes_b64": base64.b64encode(frame1).decode()},
                  {"request": 1, "kind": "chunk", "monotonic_ns": 1200000000, "bytes_b64": base64.b64encode(frame2).decode()},
                  {"request": 1, "kind": "complete", "monotonic_ns": 1500000000}]
        rows = [{"schema_version": "metrum-ai-bench-cli.request.v5", "phase": "measure", "seq": 0,
                 "error": None, "latency_s": 0.5, "ttft_s": 0.1, "ttft_source": "stream", "ttft_visible_answer_s": 0.2}]
        trace = "\n".join(map(json.dumps, events))
        data = "\n".join(map(json.dumps, rows))
        self.assertEqual(1, g1_clock_checks.evaluate(data, trace, 0)["visible_compared"])
        rows[0]["ttft_s"] = 0.5
        with self.assertRaises(ValueError):
            g1_clock_checks.evaluate("\n".join(map(json.dumps, rows)), trace, 0)
        events[-1]["monotonic_ns"] = 1700000000
        with self.assertRaises(ValueError):
            g1_clock_checks.evaluate(data, "\n".join(map(json.dumps, events)), 0)


class DefaultCapTests(unittest.TestCase):
    def test_warning_and_full_usage_required(self):
        data, trace = ReasoningEvaluatorTests().fixture()
        rows = [json.loads(line) for line in data.splitlines()]
        rows[-1]["ttft_s"] = {"p50": 0.1}
        rows[-1]["config"] = {"body_template": {"max_tokens": 512, "chat_template_kwargs": {"enable_thinking": True}}}
        data = "\n".join(map(json.dumps, rows))
        warning = "warning: 64/64 measured requests exhausted the token budget on reasoning without an answer"
        self.assertEqual(1.0, d_default_checks.evaluate(data, trace, warning, 0)["truncation_fraction"])
        with self.assertRaises(ValueError):
            d_default_checks.evaluate(data, trace, "no warning", 0)
        with self.assertRaises(ValueError):
            d_default_checks.evaluate(data.replace('"max_tokens": 512', '"max_tokens": 128'), trace, warning, 0)
        with self.assertRaises(ValueError):
            d_default_checks.evaluate("", "", "required: --max-tokens", 2)


class SchemaEvaluatorTests(unittest.TestCase):
    def fixture(self):
        data, trace = G6EvaluatorTests().fixture()
        rows = [json.loads(line) for line in data.splitlines()]
        telemetry = [{"kind": "run", "schema_version": "metrum-ai-bench-cli.telemetry.v3"}]
        for seq, row in enumerate(rows[:-1]):
            row.update({"seq": seq, "ttft_s": 0.1, "ttft_visible_answer_s": 0.2, "ttft_source": "stream",
                        "outcome": "answer", "answer_completed": True})
            telemetry.append({"kind": "request", **{key: row[key] for key in
                ("seq", "ttft_s", "ttft_visible_answer_s", "ttft_source", "outcome", "answer_completed")}})
        strategic = json.dumps({"schema_version": "metrum-ai-bench-cli.strategic.v3", "points": [{}]})
        csv = "seq,ttft_s,ttft_source,ttft_visible_answer_s,outcome,answer_completed\n1,0.1,stream,0.2,answer,true\n"
        ndjson = '\n'.join(map(json.dumps, [telemetry[0], {"kind": "request", "seq": 1, "ttft_s": 0.1,
            "ttft_source": "stream", "ttft_visible_answer_s": 0.2, "outcome": "answer", "answer_completed": True}]))
        return ('\n'.join(map(json.dumps, rows)), '\n'.join(map(json.dumps, telemetry)), trace, strategic, csv, ndjson)

    def test_live_schema_and_cross_export_consistency(self):
        fixture = self.fixture()
        self.assertEqual(64, schema_checks.evaluate(*fixture)["llm_answered"])
        bad = list(fixture)
        bad[-1] = bad[-1].replace('"outcome": "answer"', '"outcome": "wrong"')
        with self.assertRaises(ValueError):
            schema_checks.evaluate(*bad)
        bad = list(fixture)
        bad[0] = bad[0].replace('"metrum-ai-bench-cli.summary.v5"', '"metrum-ai-bench-cli.summary.v4"')
        with self.assertRaises(ValueError):
            schema_checks.evaluate(*bad)


class L1EvaluatorTests(unittest.TestCase):
    def test_timeout_terminates_subprocess_group(self):
        with tempfile.TemporaryDirectory() as temp:
            stdout, stderr = Path(temp) / "out", Path(temp) / "err"
            status = l1_runner.call([sys.executable, "-c", "import time; time.sleep(30)"], stdout, stderr, 0.1)
            self.assertEqual(124, status)
            self.assertIn("timed out", stderr.read_text())

    def data(self, successes=16):
        rows = [{"schema_version": "metrum-ai-bench-cli.request.v5", "phase": "measure",
                 "error": None if index < successes else {"kind": "error"}} for index in range(16)]
        rows.append({"schema_version": "metrum-ai-bench-cli.summary.v5", "attempted": 16,
                     "successes": successes, "config": {"common": {"require_sut": True}}, "sut": {"model": "test"}})
        return "\n".join(json.dumps(row) for row in rows)

    def test_l1_requires_all_success_and_both_exit_codes(self):
        self.assertTrue(l1_runner.evaluate(self.data(), 0, 0)[0])
        self.assertFalse(l1_runner.evaluate(self.data(15), 0, 0)[0])
        self.assertFalse(l1_runner.evaluate(self.data(), 1, 0)[0])
        self.assertFalse(l1_runner.evaluate(self.data(), 0, 1)[0])
        self.assertFalse(l1_runner.evaluate("", 0, 0)[0])


class ProxyTests(unittest.TestCase):
    def test_forwards_actual_response_and_records_exact_bytes(self):
        body = b'data: {"choices":[]}\n\ndata: [DONE]\n\n'
        class Upstream(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass
            def do_POST(self):
                self.rfile.read(int(self.headers["Content-Length"]))
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.send_header("Content-Type", "text/event-stream")
                self.end_headers()
                self.wfile.write(body)
        with tempfile.TemporaryDirectory() as temp:
            recorder = trace_proxy.Recorder(Path(temp) / "trace.jsonl")
            upstream = http.server.ThreadingHTTPServer(("localhost", 0), Upstream)
            proxy = http.server.ThreadingHTTPServer(("localhost", 0), trace_proxy.make_handler(upstream.server_port, recorder))
            threads = [threading.Thread(target=server.serve_forever, daemon=True) for server in (upstream, proxy)]
            for thread in threads:
                thread.start()
            try:
                client = http.client.HTTPConnection("localhost", proxy.server_port, timeout=5)
                client.request("POST", "/v1/chat/completions", body=b"{}", headers={"Content-Type": "application/json"})
                response = client.getresponse()
                self.assertEqual(200, response.status)
                self.assertEqual(body, response.read())
                client.close()
            finally:
                proxy.shutdown()
                upstream.shutdown()
                for thread in threads:
                    thread.join(timeout=5)
                proxy.server_close()
                upstream.server_close()
                recorder.stream.close()
            events = [json.loads(line) for line in (Path(temp) / "trace.jsonl").read_text().splitlines()]
            captured = b"".join(base64.b64decode(event["bytes_b64"]) for event in events if event["kind"] == "chunk")
            self.assertEqual(body, captured)
            self.assertEqual("complete", events[-1]["kind"])


    def test_fault_proxy_retains_original_and_forwarded_bytes(self):
        body = b'data: {"choices":[],"usage":{"prompt_tokens":2,"completion_tokens":3}}\n\ndata: [DONE]\n\n'
        class Upstream(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass
            def do_POST(self):
                self.rfile.read(int(self.headers["Content-Length"]))
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "trace.jsonl"
            recorder = trace_proxy.Recorder(path)
            upstream = http.server.ThreadingHTTPServer(("localhost", 0), Upstream)
            proxy = http.server.ThreadingHTTPServer(("localhost", 0), trace_proxy.make_handler(upstream.server_port, recorder, "remove_usage", 1))
            threads = [threading.Thread(target=s.serve_forever, daemon=True) for s in (upstream, proxy)]
            for thread in threads:
                thread.start()
            try:
                client = http.client.HTTPConnection("localhost", proxy.server_port, timeout=5)
                client.request("POST", "/v1/chat/completions", body=b"{}")
                forwarded = client.getresponse().read()
                client.close()
            finally:
                proxy.shutdown(); upstream.shutdown()
                for thread in threads:
                    thread.join(timeout=5)
                proxy.server_close(); upstream.server_close(); recorder.stream.close()
            events = [json.loads(line) for line in path.read_text().splitlines()]
            original = b"".join(base64.b64decode(e["bytes_b64"]) for e in events if e["kind"] == "chunk")
            captured = b"".join(base64.b64decode(e["bytes_b64"]) for e in events if e["kind"] == "forwarded_chunk")
            self.assertEqual(body, original)
            self.assertEqual(forwarded, captured)
            self.assertNotIn(b'"usage"', forwarded)
            self.assertTrue(any(e["kind"] == "transformation" and e["mode"] == "remove_usage" for e in events))


    def test_restart_retry_forwards_only_upstream_response(self):
        body = b'data: [DONE]\n\n'
        posts = []
        class Upstream(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass
            def do_GET(self):
                self.send_response(200 if self.path == '/health' else 404)
                self.send_header('Content-Length', '0'); self.end_headers()
            def do_POST(self):
                posts.append(self.rfile.read(int(self.headers["Content-Length"])))
                self.send_response(200); self.send_header("Content-Length", str(len(body)))
                self.end_headers(); self.wfile.write(body)
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "trace.jsonl"
            recorder = trace_proxy.Recorder(path)
            with socket.socket() as reserve:
                reserve.bind(("localhost", 0))
                port = reserve.getsockname()[1]
            proxy = http.server.ThreadingHTTPServer(("localhost", 0), trace_proxy.make_handler(port, recorder, retry_seconds=3))
            proxy_thread = threading.Thread(target=proxy.serve_forever, daemon=True)
            proxy_thread.start()
            upstream = http.server.ThreadingHTTPServer(("localhost", port), Upstream, bind_and_activate=False)
            upstream_thread = None
            output = []
            def request():
                client = http.client.HTTPConnection("localhost", proxy.server_port, timeout=5)
                try:
                    client.request("POST", "/v1/chat/completions", body=b"{}")
                    output.append(client.getresponse().read())
                finally:
                    client.close()
            client_thread = threading.Thread(target=request, daemon=True)
            client_thread.start()
            try:
                deadline = time.monotonic() + 2
                while time.monotonic() < deadline and '"kind":"upstream_wait"' not in path.read_text():
                    time.sleep(.02)
                self.assertIn('"kind":"upstream_wait"', path.read_text())
                upstream.server_bind(); upstream.server_activate()
                upstream_thread = threading.Thread(target=upstream.serve_forever, daemon=True); upstream_thread.start()
                client_thread.join(timeout=5)
                self.assertFalse(client_thread.is_alive())
                self.assertEqual([body], output)
            finally:
                proxy.shutdown(); proxy.server_close(); proxy_thread.join(timeout=5)
                if upstream_thread is not None:
                    upstream.shutdown(); upstream_thread.join(timeout=5)
                upstream.server_close(); recorder.stream.close()
            events = [json.loads(line) for line in path.read_text().splitlines()]
            self.assertTrue(any(e["kind"] == "upstream_wait" for e in events))
            self.assertTrue(any(e["kind"] == "upstream_ready" for e in events))
            self.assertEqual([b"{}"], posts)
            self.assertEqual("complete", events[-1]["kind"])

    def test_proxy_delay_preserves_real_response_bytes(self):
        body = b'data: {"choices":[]}\n\n'
        class Upstream(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass
            def do_POST(self):
                self.rfile.read(int(self.headers['Content-Length']))
                self.send_response(200); self.send_header('Content-Length', str(len(body))); self.end_headers()
                self.wfile.write(body)
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'trace.jsonl'
            recorder = trace_proxy.Recorder(path)
            upstream = http.server.ThreadingHTTPServer(('localhost', 0), Upstream)
            proxy = http.server.ThreadingHTTPServer(('localhost', 0), trace_proxy.make_handler(upstream.server_port, recorder, delay_request=1, delay_seconds=.05))
            threads = [threading.Thread(target=s.serve_forever, daemon=True) for s in (upstream, proxy)]
            for thread in threads:
                thread.start()
            try:
                client = http.client.HTTPConnection('localhost', proxy.server_port, timeout=5)
                client.request('POST', '/v1/chat/completions', body=b'{}')
                self.assertEqual(body, client.getresponse().read()); client.close()
            finally:
                proxy.shutdown(); upstream.shutdown()
                for thread in threads:
                    thread.join(timeout=5)
                proxy.server_close(); upstream.server_close(); recorder.stream.close()
            events = [json.loads(line) for line in path.read_text().splitlines()]
            self.assertEqual(1, sum(e['kind'] == 'delay' for e in events))
            self.assertEqual(body, b''.join(base64.b64decode(e['bytes_b64']) for e in events if e['kind'] == 'chunk'))
            self.assertEqual(body, b''.join(base64.b64decode(e['bytes_b64']) for e in events if e['kind'] == 'forwarded_chunk'))

    def test_caps_modify_only_real_request_caps(self):
        observed = []
        class Upstream(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass
            def do_POST(self):
                observed.append(json.loads(self.rfile.read(int(self.headers["Content-Length"]))))
                self.send_response(200); self.send_header("Content-Length", "2"); self.end_headers(); self.wfile.write(b"{}")
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "trace.jsonl"
            recorder = trace_proxy.Recorder(path)
            upstream = http.server.ThreadingHTTPServer(("localhost", 0), Upstream)
            proxy = http.server.ThreadingHTTPServer(("localhost", 0), trace_proxy.make_handler(upstream.server_port, recorder, request_caps=[128, 4096]))
            threads = [threading.Thread(target=s.serve_forever, daemon=True) for s in (upstream, proxy)]
            for thread in threads:
                thread.start()
            try:
                for _ in range(2):
                    client = http.client.HTTPConnection("localhost", proxy.server_port, timeout=5)
                    client.request("POST", "/v1/chat/completions", body=json.dumps({"max_tokens": 512, "model": "test"}))
                    self.assertEqual(b"{}", client.getresponse().read()); client.close()
            finally:
                proxy.shutdown(); upstream.shutdown()
                for thread in threads:
                    thread.join(timeout=5)
                proxy.server_close(); upstream.server_close(); recorder.stream.close()
            self.assertEqual([128, 4096], [row["max_tokens"] for row in observed])
            self.assertTrue(all(row["model"] == "test" for row in observed))
            events = [json.loads(line) for line in path.read_text().splitlines()]
            self.assertEqual([128, 4096], [event["forwarded_cap"] for event in events if event["kind"] == "request_transform"])


class ResultWriterTests(unittest.TestCase):
    def test_lane_attempt_is_preserved(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.dict(os.environ, {"CERT_ATTEMPT": "2"}):
            path = result_writer.write_record(Path(temp), {"ID": "BC-LIVE-001", "result": "FAIL", "candidate_sha": "a" * 40, "attempt": 1})
            self.assertEqual(2, json.loads(path.read_text())["attempt"])
            with mock.patch.dict(os.environ, {"CERT_ATTEMPT": "0"}), self.assertRaises(ValueError):
                result_writer.write_record(Path(temp), {"ID": "BC-LIVE-002", "result": "FAIL", "candidate_sha": "a" * 40})

    def test_atomic_write_requires_evidence_and_refuses_overwrite(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "evidence.txt").write_text("observation", encoding="utf-8")
            record = {"ID": "BC-LIVE-001", "result": "PASS", "candidate_sha": "a" * 40,
                      "observed_values": {"successes": 16}, "thresholds": {"successes": 16},
                      "versions": {"server": "pinned"}, "gpu_class": "L40S", "driver": "test",
                      "owner": "tester", "evidence_link": "https://example.test/run", "attempt": 1,
                      "start_utc": "2026-11-05T00:00:00Z", "end_utc": "2026-11-05T00:01:00Z",
                      "evidence_paths": ["evidence.txt"]}
            path = result_writer.write_record(root, record)
            self.assertEqual("PASS", json.loads(path.read_text())["result"])
            with self.assertRaises(FileExistsError):
                result_writer.write_record(root, record)
            with self.assertRaises(ValueError):
                result_writer.write_record(root, {**record, "ID": "BC-LIVE-002", "evidence_paths": ["../escape"]})
            with self.assertRaises(ValueError):
                result_writer.write_record(root, {**record, "ID": "BC-LIVE-002", "thresholds": {}})


if __name__ == "__main__":
    unittest.main()
