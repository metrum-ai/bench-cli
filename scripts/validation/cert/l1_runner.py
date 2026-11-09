#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""L1 candidate live LLM smoke and evidence evaluator."""
import argparse
import datetime as dt
import json
import os
import signal
import subprocess
from pathlib import Path

import manifest_tool
import result_writer

ROOT = Path(__file__).resolve().parents[3]


def _terminate(_signal, _frame):
    raise SystemExit(143)


# Raising lets proxy and child-process finally blocks execute before the lane cleans up.
signal.signal(signal.SIGTERM, _terminate)


def evaluate(data, process_exit, assert_exit):
    rows = [json.loads(line) for line in data.splitlines() if line.strip()]
    requests = [row for row in rows if "request.v" in row.get("schema_version", "") and row.get("phase", "measure") == "measure"]
    summaries = [row for row in rows if "summary.v" in row.get("schema_version", "")]
    summary = summaries[-1] if summaries else {}
    successes = sum(row.get("error") is None for row in requests)
    require_sut = summary.get("config", {}).get("common", {}).get("require_sut") is True
    sut_present = summary.get("sut") is not None
    observed = {"attempted": len(requests), "successes": successes,
                "summary_attempted": summary.get("attempted"), "summary_successes": summary.get("successes"),
                "require_sut": require_sut, "sut_present": sut_present,
                "process_exit": process_exit, "headline_exit": assert_exit}
    passed = (process_exit == 0 and assert_exit == 0 and len(requests) == 16 and successes == 16 and
              summary.get("attempted") == 16 and summary.get("successes") == 16 and require_sut and sut_present)
    return passed, observed


def call(command, stdout, stderr, timeout, env=None):
    try:
        process = subprocess.Popen(command, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   env=env, start_new_session=True)
    except OSError as exc:
        stdout.write_bytes(b"")
        stderr.write_text(str(exc), encoding="utf-8")
        return 127
    try:
        try:
            out, err = process.communicate(timeout=timeout)
            code = process.returncode
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                out, err = process.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                out, err = process.communicate()
            err += b"\nprocess timed out\n"
            code = 124
        stdout.write_bytes(out)
        stderr.write_bytes(err)
        return code
    finally:
        if process.poll() is None:
            try:
                os.killpg(process.pid, signal.SIGTERM)
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
            except ProcessLookupError:
                process.wait()


def smoke_invocation(args, smoke_dir):
    """Assemble the actual L1 smoke argv and environment additions."""
    command = ["cargo", "--locked", "xtask", "local-smoke", "llm", "--url", f"http://localhost:{args.port}",
               "--model", args.model, "--sut", str(args.sut), "--out", str(smoke_dir), "--num-requests", "16",
               "--concurrency", "2", "--warmup", "0", "--seed", "42", "--root", str(ROOT)]
    additions = {"BENCH_BIN_DIR": str(args.candidate.parent), "PROMPT_LOCAL_JSONL": str(args.prompt),
                 "MAX_TOKENS": "128", "LLM_EXTRA_BODY": '{"chat_template_kwargs":{"enable_thinking":false}}',
                 "MIN_SUCCESS_RATIO": "1.0"}
    return command, additions


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("out", "candidate", "baseline", "manifest", "sut", "prompt"):
        parser.add_argument("--" + name, type=Path, required=True)
    for name in ("sha", "model", "revision", "image", "start-utc", "owner", "evidence-link"):
        parser.add_argument("--" + name, required=True)
    parser.add_argument("--port", type=int, required=True)
    args = parser.parse_args()
    evidence = args.out / "evidence"
    smoke_dir = evidence / "l1"
    data_log = smoke_dir / "results.jsonl"
    stdout, stderr = evidence / "l1-stdout.txt", evidence / "l1-stderr.txt"
    assert_out, assert_err = evidence / "l1-headline.txt", evidence / "l1-headline-error.txt"
    command, additions = smoke_invocation(args, smoke_dir)
    env = os.environ.copy()
    env.update(additions)
    (evidence / "l1-command.json").write_text(json.dumps({"argv": command, "bench_bin": str(args.candidate),
        "prompt": str(args.prompt), "max_tokens": 128, "thinking": False}, indent=2) + "\n", encoding="utf-8")
    process_exit = call(command, stdout, stderr, 900, env=env)
    assert_exit = call(["cargo", "--locked", "xtask", "assert-headline", "llm", str(data_log)], assert_out, assert_err, 300)
    try:
        passed, observed = evaluate(data_log.read_text(encoding="utf-8"), process_exit, assert_exit)
    except (OSError, ValueError, TypeError, AttributeError) as exc:
        passed, observed = False, {"process_exit": process_exit, "headline_exit": assert_exit, "error": str(exc)}
    try:
        served = json.loads((evidence / "models.json").read_text(encoding="utf-8"))
        ids = [row.get("id") for row in served.get("data", [])]
        observed["served_model_match"] = args.model in ids
        passed = passed and observed["served_model_match"]
    except (OSError, ValueError, TypeError, AttributeError) as exc:
        observed["served_model_error"] = str(exc)
        passed = False
    gpu_line = (evidence / "gpu.csv").read_text(encoding="utf-8").strip().split(",", 1)
    record = {"ID": "BC-LIVE-001", "result": "PASS" if passed else "FAIL", "candidate_sha": args.sha,
              "attempt": 1, "start_utc": args.start_utc,
              "end_utc": dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z"),
              "owner": args.owner, "evidence_link": args.evidence_link,
              "gpu_class": gpu_line[0].strip(), "driver": gpu_line[1].strip() if len(gpu_line) > 1 else "",
              "versions": {"server_image": args.image, "model": args.model, "model_revision": args.revision,
                           "candidate_binary_sha256": manifest_tool.sha256_file(args.candidate),
                           "baseline_binary_sha256": manifest_tool.sha256_file(args.baseline),
                           "manifest_sha256": manifest_tool.sha256_file(args.manifest)},
              "observed_values": observed,
              "thresholds": {"measured_attempts": 16, "success_ratio": 1.0, "require_sut": True,
                             "candidate_exit": 0, "headline_exit": 0, "served_model_match": True},
              "evidence_paths": ["evidence/l1-command.json", "evidence/l1-stdout.txt", "evidence/l1-stderr.txt",
                                 "evidence/l1-headline.txt", "evidence/l1-headline-error.txt", "evidence/models.json",
                                 "evidence/gpu.csv", "evidence/server.log", "evidence/binary-hashes.txt"]}
    # Preserve the internally generated input, original selector command, both
    # independent generations and their pre-measurement binding receipt.
    record['evidence_paths'] = sorted(set(record['evidence_paths']) | {
        str(path.relative_to(args.out)) for path in smoke_dir.rglob('*') if path.is_file()})
    result_writer.write_record(args.out, record)
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
