#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Exercise mixed real streams across actual server termination and restart."""
import argparse
import datetime as dt
import json
import os
import signal
import subprocess
import sys
import time
from pathlib import Path

import g3_checks
import g6_counter_runner
import l1_runner
import manifest_tool
import result_writer

ROOT = Path(__file__).resolve().parents[3]
# Fixed before G3 attempt 3; previous 150s run and its failure are retained.
E2E_SLO_SECONDS = 60


def events(path):
    try:
        return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    except (OSError, json.JSONDecodeError):
        return []


def ready_to_interrupt(order, trace, data):
    stream = events(trace)
    started = {e.get("request") for e in stream if e.get("kind") == "chunk"}
    finished = {e.get("request") for e in stream if e.get("kind") in ("complete", "error")}
    if not started - finished:
        return False
    if order == "error-first":
        return not finished
    rows = [r for r in events(data) if ".request." in r.get("schema_version", "")]
    return any(r.get("answer_completed") is True and r.get("error") is None for r in rows) and any(
        r.get("outcome") == "reasoning_truncated" for r in rows)


def stop_client(client):
    if client.poll() is not None:
        return
    try:
        os.killpg(client.pid, signal.SIGTERM)
        client.wait(timeout=10)
    except ProcessLookupError:
        client.wait(timeout=10)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(client.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        client.wait(timeout=10)


def run_cell(args, order, label="g3", extra_flags=(), concurrency=4, count=16):
    evidence = args.out / "evidence"
    prefix = f"{label}-{order}"
    trace, data = evidence / f"{prefix}-trace.jsonl", evidence / f"{prefix}-requests.jsonl"
    proxyerr, stdout, stderr = (evidence / f"{prefix}-{suffix}" for suffix in ("proxy-error.txt", "stdout.txt", "stderr.txt"))
    container = (evidence / "container-id.txt").read_text(encoding="utf-8").strip()
    if not container or any(c not in "0123456789abcdef" for c in container):
        raise ValueError("invalid pinned serving container identity")
    command = [str(args.candidate), "--url", f"http://localhost:{args.proxy_port}/v1/chat/completions",
               "--api-key", "dummy", "--scenario", prefix, "--num-requests", str(count), "--concurrency", str(concurrency),
               "--prompts", str(args.prompt), "--mode", "chat", "--streaming", "--model", args.model,
               "--data-log", str(data), "--max-tokens", "4096", "--warmup-requests", "0", "--seed", "42",
               "--temperature", "0", "--request-timeout", "600", "--slo", f"e2e={E2E_SLO_SECONDS}",
               "--extra-body-json", '{"chat_template_kwargs":{"enable_thinking":true}}', *extra_flags]
    proxycmd = [sys.executable, str(Path(__file__).with_name("trace_proxy.py")), "--listen-port", str(args.proxy_port),
                "--upstream-port", str(args.port), "--out", str(trace), "--request-caps", "128,4096", "--retry-seconds", "240"]
    commands = {"benchmark": command, "proxy": proxycmd}
    if order != "control":
        commands.update({"interrupt": ["docker", "kill", container], "restart": ["docker", "start", container]})
    (evidence / f"{prefix}-commands.json").write_text(json.dumps(commands, indent=2) + "\n")
    interruption = evidence / f"{prefix}-interruption.json"
    client = proxy = None
    status = None
    with proxyerr.open("wb") as perr, stdout.open("wb") as out, stderr.open("wb") as err:
        try:
            proxy = subprocess.Popen(proxycmd, stdout=subprocess.DEVNULL, stderr=perr, cwd=ROOT)
            g6_counter_runner.wait_for_proxy(args.proxy_port, proxy)
            client = subprocess.Popen(command, stdout=out, stderr=err, cwd=ROOT, start_new_session=True)
            if order != "control":
                deadline = time.monotonic() + 900
                while time.monotonic() < deadline and client.poll() is None:
                    if ready_to_interrupt(order, trace, data):
                        break
                    time.sleep(.05)
                else:
                    raise ValueError("required pre-interruption populations or active server stream missing")
                before = time.monotonic_ns()
                stop = subprocess.run(["docker", "kill", container], capture_output=True, text=True, timeout=30)
                after = time.monotonic_ns()
                restart = subprocess.run(["docker", "start", container], capture_output=True, text=True, timeout=30)
                interruption.write_text(json.dumps({"order": order, "kill_start_ns": before, "kill_end_ns": after,
                                                    "kill_exit": stop.returncode, "start_exit": restart.returncode,
                                                    "kill_stdout": stop.stdout, "kill_stderr": stop.stderr,
                                                    "start_stdout": restart.stdout, "start_stderr": restart.stderr}, indent=2) + "\n")
                if stop.returncode or restart.returncode:
                    raise ValueError("real server kill/restart failed")
            try:
                status = client.wait(timeout=2400)
            except subprocess.TimeoutExpired:
                stop_client(client)
                raise ValueError("benchmark timed out after real interruption")
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                ev = events(trace)
                sent = {e.get("request") for e in ev if e.get("kind") == "send"}
                done = {e.get("request") for e in ev if e.get("kind") in ("complete", "error")}
                if sent and sent == done:
                    break
                time.sleep(.1)
        finally:
            if client is not None and client.poll() is None:
                stop_client(client)
            if proxy is not None:
                proxy.terminate()
                try:
                    proxy.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    proxy.kill(); proxy.wait(timeout=10)
    stream = events(trace)
    if order != "control":
        if not interruption.is_file():
            raise ValueError("real server interruption record missing")
        interruption_data = json.loads(interruption.read_text(encoding="utf-8"))
        if not any(e.get("kind") == "error" and interruption_data["kill_start_ns"] <= e.get("monotonic_ns", 0) <= interruption_data["kill_end_ns"] + 120_000_000_000 for e in stream):
            raise ValueError("server interruption produced no temporally matched upstream failure")
    elif status != 0:
        raise ValueError(f"complete-usage control exited {status}")
    return data.read_text(encoding="utf-8"), trace.read_text(encoding="utf-8"), status


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
    gpu = (evidence / "gpu.csv").read_text(encoding="utf-8").strip().split(",", 1)
    outcomes, cells = {}, {}
    for order in ("control", "error-first", "error-last"):
        try:
            data, trace, exit_code = run_cell(args, order)
            cells[order] = (data, trace)
            check = g3_checks.verify_complete_control(data, trace) if order == "control" else g3_checks.verify_window(data, trace, order)
            outcomes[order] = {"window": check, "exit_code": exit_code}
        except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, subprocess.SubprocessError) as exc:
            outcomes[order] = {"error": str(exc)}
    # Both ordering cells must satisfy the window contract. Goodput is separately
    # evaluated against the retained error-last case; no missing population passes.
    for case in ("BC-GPU-008", "BC-GPU-009"):
        try:
            if case == "BC-GPU-008":
                if any("error" in outcomes[order] for order in ("control", "error-first", "error-last")):
                    raise ValueError("complete-usage control or real interruption orderings failed")
                observed = {"cells": outcomes}
            else:
                data, trace = cells["error-last"]
                observed = {"goodput": g3_checks.verify_goodput(data, trace, E2E_SLO_SECONDS), "window_cell": outcomes["error-last"]}
                if "error" in outcomes["error-last"]:
                    raise ValueError("real error-last window verification failed")
            passed = True
        except (ValueError, KeyError, TypeError) as exc:
            observed, passed = {"error": str(exc), "cells": outcomes}, False
        files = ["evidence/" + x for x in ("gpu.csv", "server.log", "binary-hashes.txt", "container-id.txt")]
        files += [str(p.relative_to(args.out)) for p in evidence.glob("g3-*") if p.is_file()]
        record = {"ID": case, "result": "PASS" if passed else "FAIL", "candidate_sha": args.sha, "attempt": 1,
                  "start_utc": args.start_utc, "end_utc": dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z"),
                  "owner": args.owner, "evidence_link": args.evidence_link, "gpu_class": gpu[0].strip(),
                  "driver": gpu[1].strip() if len(gpu) > 1 else "",
                  "versions": {"server_image": args.image, "model": args.model, "model_revision": args.revision,
                               "candidate_binary_sha256": manifest_tool.sha256_file(args.candidate),
                               "baseline_binary_sha256": manifest_tool.sha256_file(args.baseline),
                               "manifest_sha256": manifest_tool.sha256_file(args.manifest)},
                  "observed_values": observed, "thresholds": {"window_tolerance_s": .05, "rate_tolerance_pct": 1,
                  "e2e_slo_s": E2E_SLO_SECONDS, "server_kill_restart_required": True}, "evidence_paths": files}
        result_writer.write_record(args.out, record)
        outcomes[case] = passed
    return 0 if outcomes["BC-GPU-008"] and outcomes["BC-GPU-009"] else 1


if __name__ == "__main__":
    sys.exit(main())
