#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Run the G6 live server counter case. Schema migration is a separate case."""
import argparse
import datetime as dt
import json
import socket
import subprocess
import sys
import time
from pathlib import Path
from urllib.request import urlopen

import g6_checks
import l1_runner
import manifest_tool
import result_writer

ROOT = Path(__file__).resolve().parents[3]


def wait_for_proxy(port, process):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise ValueError("response recorder exited before becoming ready")
        try:
            with socket.create_connection(("localhost", port), timeout=1):
                return
        except OSError:
            time.sleep(0.1)
    raise ValueError("response recorder readiness timeout")


def snapshot(port, path):
    with urlopen(f"http://localhost:{port}/metrics", timeout=10) as response:
        data = response.read(10_000_001)
    if len(data) > 10_000_000:
        raise ValueError("metrics response too large")
    path.write_bytes(data)
    return data.decode("utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("out", "candidate", "baseline", "manifest", "prompt"):
        parser.add_argument("--" + name, type=Path, required=True)
    for name in ("sha", "model", "revision", "image", "start-utc", "owner", "evidence-link"):
        parser.add_argument("--" + name, required=True)
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--proxy-port", type=int, required=True)
    args = parser.parse_args()
    evidence = args.out / "evidence"
    trace = evidence / "g6-trace.jsonl"
    data_log = evidence / "g6-results.jsonl"
    before_path, after_path = evidence / "g6-metrics-before.txt", evidence / "g6-metrics-after.txt"
    stdout, stderr = evidence / "g6-stdout.txt", evidence / "g6-stderr.txt"
    proxy_stdout, proxy_stderr = evidence / "g6-proxy-stdout.txt", evidence / "g6-proxy-stderr.txt"
    command = [str(args.candidate), "--url", f"http://localhost:{args.proxy_port}/v1/chat/completions",
               "--api-key", "dummy", "--scenario", "cert-counter-check", "--num-requests", "64",
               "--concurrency", "16", "--prompts", str(args.prompt), "--mode", "chat", "--streaming",
               "--model", args.model, "--data-log", str(data_log), "--max-tokens", "128", "--warmup-requests", "0",
               "--seed", "42", "--extra-body-json", '{"chat_template_kwargs":{"enable_thinking":true}}']
    (evidence / "g6-command.json").write_text(json.dumps(command, indent=2) + "\n", encoding="utf-8")
    observed = {}
    passed = False
    process = None
    try:
        with proxy_stdout.open("wb") as pout, proxy_stderr.open("wb") as perr:
            process = subprocess.Popen([sys.executable, str(Path(__file__).with_name("trace_proxy.py")),
                                        "--listen-port", str(args.proxy_port), "--upstream-port", str(args.port),
                                        "--out", str(trace)], stdout=pout, stderr=perr, cwd=ROOT)
            try:
                wait_for_proxy(args.proxy_port, process)
                before = snapshot(args.port, before_path)
                exit_code = l1_runner.call(command, stdout, stderr, 1800)
                # The client may finish before the recorder has flushed all terminal events.
                deadline = time.monotonic() + 15
                while time.monotonic() < deadline:
                    try:
                        if len(g6_checks.server_usage(trace.read_text(encoding="utf-8"))) == 64:
                            break
                    except (OSError, ValueError, KeyError, TypeError):
                        pass
                    if process.poll() is not None:
                        raise ValueError("response recorder exited early")
                    time.sleep(0.1)
                after = snapshot(args.port, after_path)
                if exit_code != 0:
                    raise ValueError(f"candidate exited {exit_code}")
                observed = g6_checks.evaluate(data_log.read_text(encoding="utf-8"),
                                              trace.read_text(encoding="utf-8"), before, after, args.model)
                passed = True
            finally:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=10)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as exc:
        observed = {"error": str(exc)}
    gpu_line = (evidence / "gpu.csv").read_text(encoding="utf-8").strip().split(",", 1)
    evidence_files = ["g6-command.json", "g6-proxy-stdout.txt", "g6-proxy-stderr.txt", "gpu.csv", "server.log", "binary-hashes.txt"]
    evidence_files += [name for name in ("g6-trace.jsonl", "g6-results.jsonl", "g6-metrics-before.txt",
                                         "g6-metrics-after.txt", "g6-stdout.txt", "g6-stderr.txt")
                       if (evidence / name).is_file()]
    record = {"ID": "BC-COMP-003", "result": "PASS" if passed else "FAIL", "candidate_sha": args.sha,
              "attempt": 1, "start_utc": args.start_utc,
              "end_utc": dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z"),
              "owner": args.owner, "evidence_link": args.evidence_link,
              "gpu_class": gpu_line[0].strip(), "driver": gpu_line[1].strip() if len(gpu_line) > 1 else "",
              "versions": {"server_image": args.image, "model": args.model, "model_revision": args.revision,
                           "candidate_binary_sha256": manifest_tool.sha256_file(args.candidate),
                           "baseline_binary_sha256": manifest_tool.sha256_file(args.baseline),
                           "manifest_sha256": manifest_tool.sha256_file(args.manifest)},
              "observed_values": observed,
              "thresholds": {"measured_requests": 64, "reasoning_requests_min": 1, "counter_tolerance_pct": 5,
                             "counter_absolute_tolerance_tokens": 1, "rate_relative_tolerance_pct": 1},
              "evidence_paths": ["evidence/" + name for name in evidence_files]}
    result_writer.write_record(args.out, record)
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
