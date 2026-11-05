#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Measure both real-stream TTFT definitions using pinned vLLM bench serve.

Implements prospectively approved r3; GPU-development outcomes are not certification.
"""
import argparse
import datetime as dt
import hashlib
import json
import os
import secrets
import subprocess
import sys
import time
from pathlib import Path

import comp_reference_checks as checks
import g6_counter_runner
import l1_runner
import manifest_tool
import result_writer

ROOT = Path(__file__).resolve().parents[3]


def pinned_file(value, label):
    if not isinstance(value, dict) or not isinstance(value.get("path"), str) or not isinstance(value.get("sha256"), str):
        raise ValueError(f"{label} pin missing")
    path = Path(value["path"]).resolve()
    if manifest_tool.sha256_file(path) != value["sha256"]:
        raise ValueError(f"{label} hash differs")
    return path


def run_recorded(args, name, command):
    evidence = args.out / "evidence"
    trace = evidence / f"comp-{name}-trace.jsonl"
    proxy_err = evidence / f"comp-{name}-proxy-error.txt"
    with proxy_err.open("wb") as err:
        proxy = subprocess.Popen([sys.executable, str(Path(__file__).with_name("trace_proxy.py")),
                                  "--listen-port", str(args.proxy_port), "--upstream-port", str(args.port),
                                  "--out", str(trace), "--record-request-identity"], stdout=subprocess.DEVNULL,
                                 stderr=err, cwd=ROOT)
        try:
            g6_counter_runner.wait_for_proxy(args.proxy_port, proxy)
            code = l1_runner.call(command, evidence / f"comp-{name}-stdout.txt", evidence / f"comp-{name}-stderr.txt", 3600)
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                try:
                    events = [json.loads(line) for line in trace.read_text(encoding="utf-8").splitlines() if line.strip()]
                    sent = {e.get("request") for e in events if e.get("kind") == "send"}
                    terminal = {e.get("request") for e in events if e.get("kind") in ("complete", "error")}
                    if sent and sent == terminal:
                        break
                except (OSError, ValueError):
                    pass
                time.sleep(.1)
        finally:
            proxy.terminate()
            try:
                proxy.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proxy.kill(); proxy.wait(timeout=10)
    return code, trace


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("out", "candidate", "baseline", "manifest"):
        parser.add_argument("--" + name, type=Path, required=True)
    for name in ("sha", "model", "revision", "image", "start-utc", "owner", "evidence-link"):
        parser.add_argument("--" + name, required=True)
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--proxy-port", type=int, required=True)
    args = parser.parse_args()
    evidence = args.out / "evidence"
    gpu = (evidence / "gpu.csv").read_text(encoding="utf-8").strip().split(",", 1)
    result = {"criteria_status": "prospective r3 approved; GPU development", "candidate_exit": None, "reference_exit": None}
    passed = False
    container = "cert-ref-" + secrets.token_hex(12)
    cleanup_failed = False
    cap = 1024  # Frozen prospectively for the ShareGPT loader's 2048-token limit.
    try:
        config = json.loads(args.manifest.read_text(encoding="utf-8"))["lanes"]["D"]
        prompts = pinned_file(config["prompt_sets"]["reference"], "reference prompts")
        sharegpt = pinned_file(config["prompt_sets"]["reference_sharegpt"], "reference ShareGPT dataset")
        if config.get("comparison_cap") != cap:
            raise ValueError("frozen reference comparison cap differs")
        order_path = pinned_file(config["reference_order"], "reference order")
        order = [json.loads(line) for line in order_path.read_text(encoding="utf-8").splitlines() if line.strip()]
        dataset = [json.loads(line) for line in prompts.read_text(encoding="utf-8").splitlines() if line.strip()]
        sharegpt_rows = json.loads(sharegpt.read_text(encoding="utf-8"))
        if len(dataset) != 64 or len(order) != 64 or not isinstance(sharegpt_rows, list) or len(sharegpt_rows) != 64:
            raise ValueError("64 frozen reference prompts and slots required")
        for index, (slot, item) in enumerate(zip(order, dataset)):
            prompt = item.get("prompt")
            turns = sharegpt_rows[index].get("conversations", [])
            if not isinstance(turns, list) or len(turns) != 2 or turns[0].get("value") != prompt or turns[1].get("value") != "placeholder":
                raise ValueError("reference dataset does not reproduce the frozen prompt sequence")
            if not isinstance(prompt, str) or slot.get("seq") != index or slot.get("prompt_index") != index:
                raise ValueError("reference order does not match frozen prompt IDs")
            digest = hashlib.sha256(prompt.encode("utf-8")).hexdigest()
            if slot.get("prompt_sha256_16") != digest[:16] or slot.get("prompt_bytes") != len(prompt.encode("utf-8")):
                raise ValueError("frozen prompt fingerprint or length differs")
            slot["prompt_sha256"] = digest
        if len({slot["prompt_sha256"] for slot in order}) != 64:
            raise ValueError("reference prompt IDs must be distinct")
        cache = os.environ.get("CERT_HF_HUB_CACHE", "")
        if not cache or not Path(cache).is_dir():
            raise ValueError("pinned offline tokenizer cache is required")
        tokenizer = f"/root/.cache/huggingface/hub/models--{args.model.replace('/', '--')}/snapshots/{args.revision}"
        if not (Path(cache) / f"models--{args.model.replace('/', '--')}" / "snapshots" / args.revision).is_dir():
            raise ValueError("pinned tokenizer snapshot is missing")
        candidate_data = evidence / "comp-candidate-requests.jsonl"
        candidate = [str(args.candidate), "--url", f"http://localhost:{args.proxy_port}/v1/chat/completions",
                     "--api-key", "dummy", "--scenario", "comp-reference-candidate", "--num-requests", "64",
                     "--concurrency", "1", "--prompts", str(prompts), "--prompt-sequence", str(order_path),
                     "--mode", "chat", "--streaming", "--model", args.model, "--data-log", str(candidate_data),
                     "--max-tokens", str(cap), "--temperature", "0", "--seed", "42", "--request-timeout", "21600",
                     "--extra-body-json", '{"seed":42,"chat_template_kwargs":{"enable_thinking":true}}']
        reference = ["docker", "run", "--name", container, "--pull", "never", "--network", "host",
                     "--env", "HF_HUB_OFFLINE=1", "--env", "TRANSFORMERS_OFFLINE=1", "--env", "OPENAI_API_KEY=dummy",
                     "--mount", f"type=bind,src={Path(cache).resolve()},dst=/root/.cache/huggingface/hub,readonly",
                     "--mount", f"type=bind,src={sharegpt},dst=/inputs/prompts.json,readonly",
                     "--mount", f"type=bind,src={evidence},dst=/evidence", "--entrypoint", "vllm", args.image,
                     "bench", "serve", "--backend", "openai-chat", "--base-url", f"http://localhost:{args.proxy_port}",
                     "--endpoint", "/v1/chat/completions", "--model", args.model, "--tokenizer", tokenizer,
                     "--dataset-name", "sharegpt", "--dataset-path", "/inputs/prompts.json",
                     "--disable-shuffle", "--no-oversample", "--sharegpt-output-len", str(cap), "--num-prompts", "64",
                     "--max-concurrency", "1", "--temperature", "0", "--seed", "42", "--num-warmups", "0",
                     "--extra-body", '{"seed":42,"chat_template_kwargs":{"enable_thinking":true}}', "--disable-tqdm",
                     "--save-result", "--save-detailed", "--result-dir", "/evidence", "--result-filename", "comp-reference-result.json"]
        (evidence / "comp-commands.json").write_text(json.dumps({"candidate": candidate, "reference": reference}, indent=2) + "\n")
        code, candidate_trace = run_recorded(args, "candidate", candidate)
        result["candidate_exit"] = code
        try:
            code, reference_trace = run_recorded(args, "reference", reference)
            result["reference_exit"] = code
        finally:
            cleanup = subprocess.run(["docker", "rm", "-f", container], capture_output=True, text=True, timeout=30)
            (evidence / "comp-reference-cleanup.txt").write_text(cleanup.stdout + cleanup.stderr)
            cleanup_failed = cleanup.returncode != 0
        for name, path in (("candidate_clock_gaps", candidate_trace), ("reference_clock_gaps", reference_trace)):
            result[name] = checks.observed_clock_gaps(path.read_text(encoding="utf-8"))
        native = json.loads((evidence / "comp-reference-result.json").read_text(encoding="utf-8"))
        result["reference_native"] = {key: native.get(key) for key in ("completed", "failed", "duration", "output_throughput", "total_output_tokens")}
        if candidate_data.is_file():
            candidate_rows = [json.loads(line) for line in candidate_data.read_text(encoding="utf-8").splitlines() if line.strip()]
            measured = [row for row in candidate_rows if ".request." in row.get("schema_version", "") and row.get("phase") == "measure"]
            summary = next((row for row in candidate_rows if ".summary." in row.get("schema_version", "")), {})
            result["candidate_native"] = {"attempted": len(measured), "answered": sum(row.get("answer_completed") is True for row in measured),
                                          "completion_tokens_total": summary.get("completion_tokens_total"),
                                          "completion_tokens_per_second": summary.get("completion_tokens_per_second"),
                                          "window_seconds": summary.get("window_seconds")}
        if code != 0 or result["candidate_exit"] != 0 or cleanup_failed:
            raise ValueError("candidate, reference or reference container cleanup failed")
        result["aligned_observations"] = checks.compare(candidate_data.read_text(encoding="utf-8"),
                                                        candidate_trace.read_text(encoding="utf-8"), native,
                                                        reference_trace.read_text(encoding="utf-8"), order, args.model, cap)
        if not result["aligned_observations"]["reference_ttft_aligned"] or not result["aligned_observations"]["throughput_within_10_percent"]:
            raise ValueError("approved r3 TTFT alignment or throughput tolerance failed")
        passed = True
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, subprocess.SubprocessError) as exc:
        result["reason"] = str(exc)
        for name in ("candidate", "reference"):
            path = evidence / f"comp-{name}-trace.jsonl"
            if path.is_file():
                try:
                    result[f"{name}_clock_gaps"] = checks.observed_clock_gaps(path.read_text(encoding="utf-8"))
                except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError):
                    pass
    files = ["evidence/" + name for name in ("gpu.csv", "server.log", "binary-hashes.txt")]
    files += [str(path.relative_to(args.out)) for path in evidence.glob("comp-*") if path.is_file()]
    record = {"ID": "BC-COMP-002", "result": "PASS" if passed else "FAIL", "candidate_sha": args.sha, "attempt": 1,
              "start_utc": args.start_utc, "end_utc": dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z"),
              "owner": args.owner, "evidence_link": args.evidence_link, "gpu_class": gpu[0].strip(),
              "driver": gpu[1].strip() if len(gpu) > 1 else "", "observed_values": result,
              "thresholds": {"requests_per_tool": 64, "answers_per_tool": 64, "max_tokens": cap,
                             "throughput_tolerance_pct": 10, "first_choices_ttft_tolerance_pct": 20,
                             "first_choices_ttft_absolute_tolerance_s": .05, "approved_prospective_r3": True},
              "versions": {"server_and_reference_image": args.image, "model": args.model, "model_revision": args.revision,
                           "candidate_binary_sha256": manifest_tool.sha256_file(args.candidate),
                           "baseline_binary_sha256": manifest_tool.sha256_file(args.baseline),
                           "manifest_sha256": manifest_tool.sha256_file(args.manifest)}, "evidence_paths": files}
    result_writer.write_record(args.out, record)
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
