#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Real Qwen reasoning cells; every PASS is checked against streamed evidence."""
import argparse
import datetime as dt
import json
import subprocess
import sys
import time
from pathlib import Path

import g6_checks
import g6_counter_runner
import l1_runner
import manifest_tool
import r_checks
import result_writer

ROOT = Path(__file__).resolve().parents[3]


def run_cell(args, label, binary, count, cap, thinking, extra, concurrency=16, fault=None, field_reported=False):
    if field_reported and (count != 64 or cap != 512 or thinking is not True or extra or concurrency != 16 or fault is not None):
        raise ValueError("field-reported case requires 64 requests, thinking ON, cap 512, and no other overrides")
    evidence = args.out / "evidence"
    trace = evidence / f"{label}-trace.jsonl"
    data = evidence / f"{label}-requests.jsonl"
    stdout, stderr = evidence / f"{label}-stdout.txt", evidence / f"{label}-stderr.txt"
    command = [str(binary), "--url", f"http://localhost:{args.proxy_port}/v1/chat/completions",
               "--api-key", "dummy", "--scenario", label, "--num-requests", str(count),
               "--concurrency", str(concurrency), "--prompts", str(args.prompt), "--mode", "chat", "--streaming",
               "--model", args.model, "--data-log", str(data),
               *( ["--max-tokens", str(cap)] if cap is not None else [] ),
               *([] if field_reported else ["--warmup-requests", "0", "--seed", "42", "--temperature", "0", "--request-timeout", "600"]),
               "--debug-log", str(evidence / f"{label}-debug.log"), "--error-log", str(evidence / f"{label}-error.log"),
               "--extra-body-json", json.dumps({"chat_template_kwargs": {"enable_thinking": thinking}}), *extra]
    command_path = evidence / f"{label}-command.json"
    command_path.write_text(json.dumps(command, indent=2) + "\n", encoding="utf-8")
    proxy_err = evidence / f"{label}-proxy-error.txt"
    with proxy_err.open("wb") as stderr_file:
        proxy_command = [sys.executable, str(Path(__file__).with_name("trace_proxy.py")),
                         "--listen-port", str(args.proxy_port), "--upstream-port", str(args.port), "--out", str(trace)]
        if fault is not None:
            proxy_command += ["--fault", fault]
        process = subprocess.Popen(proxy_command, stdout=subprocess.DEVNULL, stderr=stderr_file, cwd=ROOT)
        try:
            g6_counter_runner.wait_for_proxy(args.proxy_port, process)
            exit_code = l1_runner.call(command, stdout, stderr, 3600)
            deadline = time.monotonic() + 15
            while data.is_file() and time.monotonic() < deadline:
                try:
                    if len(g6_checks.server_usage(trace.read_text(encoding="utf-8"))) == count:
                        break
                except (OSError, ValueError, KeyError, TypeError):
                    pass
                if process.poll() is not None:
                    break
                time.sleep(0.1)
        finally:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=10)
    files = ["evidence/" + path.name for path in (trace, data, stdout, stderr, command_path, proxy_err) if path.is_file()]
    return (data.read_text(encoding="utf-8") if data.is_file() else "",
            trace.read_text(encoding="utf-8") if trace.is_file() else "", exit_code, files)


def baseline_observed(data, trace, count):
    rows = [json.loads(line) for line in data.splitlines() if line.strip()]
    requests = [row for row in rows if "request.v" in row.get("schema_version", "") and row.get("phase", "measure") == "measure"]
    summaries = [row for row in rows if "summary.v" in row.get("schema_version", "")]
    if len(requests) != count or not summaries or len(g6_checks.server_usage(trace)) != count:
        raise ValueError("previous release did not emit complete comparison evidence")
    return {"attempted": count, "baseline_successes": summaries[-1].get("successes")}


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
    gpu_line = (evidence / "gpu.csv").read_text(encoding="utf-8").strip().split(",", 1)
    versions = {"server_image": args.image, "model": args.model, "model_revision": args.revision,
                "candidate_binary_sha256": manifest_tool.sha256_file(args.candidate),
                "baseline_binary_sha256": manifest_tool.sha256_file(args.baseline),
                "manifest_sha256": manifest_tool.sha256_file(args.manifest)}
    common_files = ["evidence/gpu.csv", "evidence/server.log", "evidence/binary-hashes.txt"]
    outcomes = {}

    def record(case_id, callback, thresholds):
        files = list(common_files)
        try:
            observed = callback(files)
            passed = True
        except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, subprocess.SubprocessError) as exc:
            observed, passed = {"error": str(exc)}, False
            files.extend("evidence/" + path.name for path in evidence.glob("r-*") if path.is_file() and "evidence/" + path.name not in files)
        record_data = {"ID": case_id, "result": "PASS" if passed else "FAIL", "candidate_sha": args.sha,
                       "attempt": 1, "start_utc": args.start_utc,
                       "end_utc": dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z"),
                       "owner": args.owner, "evidence_link": args.evidence_link, "gpu_class": gpu_line[0].strip(),
                       "driver": gpu_line[1].strip() if len(gpu_line) > 1 else "", "versions": versions,
                       "observed_values": observed, "thresholds": thresholds, "evidence_paths": files}
        result_writer.write_record(args.out, record_data)
        outcomes[case_id] = passed

    def run(label, binary, count, cap, thinking, flags, files):
        data, trace, exit_code, paths = run_cell(args, label, binary, count, cap, thinking, flags)
        files.extend(paths)
        return data, trace, exit_code

    def truncation(files):
        cells = {}
        for cap in (128, 512):
            for mode, flags in (("default", []), ("strict", ["--strict-reasoning-truncation"]), ("ci", ["--ci-safety"])):
                data, trace, code = run(f"r-{cap}-{mode}", args.candidate, 64, cap, True, flags, files)
                cells[f"{cap}-{mode}"] = r_checks.truncation_cell(data, trace, 64, mode != "default", code)
        return cells

    def adequate(files):
        data, trace, code = run("r-4096-candidate", args.candidate, 64, 4096, True, [], files)
        candidate = r_checks.adequate_cell(data, trace, 64, code)
        safety_data, safety_trace, safety_exit = run("r-4096-safety", args.candidate, 64, 4096, True, ["--ci-safety"], files)
        safety = r_checks.safety_cell(safety_data, safety_trace, 64, safety_exit)
        base_data, base_trace, base_exit = run("r-4096-baseline", args.baseline, 64, 4096, True, [], files)
        base = baseline_observed(base_data, base_trace, 64)
        return {"candidate": candidate, "ci_safety": safety, "baseline": {**base, "exit_code": base_exit}}

    def control(files):
        data, trace, code = run("r-control-candidate", args.candidate, 64, 4096, False, [], files)
        candidate = r_checks.control_cell(data, trace, 64, code)
        base_data, base_trace, base_exit = run("r-control-baseline", args.baseline, 64, 4096, False, [], files)
        base = baseline_observed(base_data, base_trace, 64)
        return {"candidate": candidate, "baseline": {**base, "exit_code": base_exit}}

    def live(files):
        data, trace, code = run("r-live-thinking", args.candidate, 16, 512, True, [], files)
        return r_checks.thinking_live_cell(data, trace, 16, code)

    record("BC-REASON-001", truncation, {"requests_per_cell": 64, "truncations_min": 1, "usage_coverage": 1.0, "strict_exit_nonzero": True})
    record("BC-REASON-002", adequate, {"requests": 64, "answers_min": 1, "reasoning_streams_min": 1, "usage_coverage": 1.0})
    record("BC-REASON-003", control, {"requests": 64, "answers": 64, "usage_coverage": 1.0})
    record("BC-REASON-004", live, {"requests": 16, "reasoning_streams_min": 1, "usage_coverage": 1.0})

    def hardware(_files):
        if "L40S" not in gpu_line[0] or not all(outcomes.get(case, False) for case in ("BC-REASON-001", "BC-REASON-002", "BC-REASON-003")):
            raise ValueError("second-class case requires L40S and all three reasoning cases PASS")
        return {"gpu_class": gpu_line[0].strip(), "reasoning_cases_passed": 3}

    record("BC-HW-001", hardware, {"gpu_class": "L40S", "reasoning_cases_passed": 3})
    return 0 if all(outcomes.values()) else 1


if __name__ == "__main__":
    raise SystemExit(main())
