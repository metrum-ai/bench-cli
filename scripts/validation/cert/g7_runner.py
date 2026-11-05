#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Independent 8B A/B grid; it does not modify the existing 27B perf gate."""
import argparse
import datetime as dt
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

import compare_small
import g6_counter_runner
import l1_runner
import manifest_tool
import result_writer

ROOT = Path(__file__).resolve().parents[3]


def wait_for_trace(path):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        try:
            rows = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
            started = {row["request"] for row in rows if row.get("kind") == "chunk"}
            finished = {row["request"] for row in rows if row.get("kind") == "complete"}
            if started and started == finished:
                return
        except (OSError, ValueError, KeyError):
            pass
        time.sleep(0.1)
    raise ValueError("stream recorder did not drain all responses")


def run_cell(args, config, label, binary, workload, concurrency):
    root = args.out / "evidence" / label / workload / f"c{concurrency}"
    root.mkdir(parents=True)
    prompt = Path(config["prompt_sets"][workload]["path"])
    token_cap = {"W1": 128, "W2": 512}[workload]
    meta = {"model": args.model, "model_revision": args.revision, "image": args.image,
            "workload": workload, "prompt_sha256": manifest_tool.sha256_file(prompt),
            "concurrency": concurrency, "max_tokens": token_cap, "seed": 42, "temperature": 0,
            "thinking": False, "warmup_seconds": 30, "measure_seconds": 180,
            "binary_sha256": manifest_tool.sha256_file(binary)}
    (root / "point.json").write_text(json.dumps(meta, indent=2) + "\n", encoding="utf-8")

    def command(duration, port, name):
        return [str(binary), "--scenario", f"g7-{workload}-c{concurrency}-{name}",
                "--url", f"http://localhost:{port}/v1/chat/completions", "--api-key", "dummy",
                "--num-requests", "1000000", "--concurrency", str(concurrency), "--prompts", str(prompt),
                "--mode", "chat", "--streaming", "--model", args.model, "--data-log", str(root / f"{name}.jsonl"),
                "--max-tokens", str(token_cap), "--temperature", "0", "--seed", "42", "--warmup-requests", "0",
                "--extra-body-json", json.dumps({"seed": 42, "chat_template_kwargs": {"enable_thinking": False}}),
                "--stop-after-seconds", str(duration), "--request-timeout", "600",
                "--debug-log", str(root / f"{name}-debug.log"), "--error-log", str(root / f"{name}-error.log")]

    warm = command(30, args.port, "warmup")
    (root / "warmup-command.json").write_text(json.dumps(warm, indent=2) + "\n", encoding="utf-8")
    if l1_runner.call(warm, root / "warmup-stdout.txt", root / "warmup-stderr.txt", 900) != 0:
        raise ValueError(f"{label}/{workload}/c{concurrency} warmup failed")
    trace = root / "trace.jsonl"
    with (root / "proxy-error.txt").open("wb") as proxy_err:
        proxy = subprocess.Popen([sys.executable, str(Path(__file__).with_name("trace_proxy.py")),
                                  "--listen-port", str(args.proxy_port), "--upstream-port", str(args.port),
                                  "--out", str(trace)], stdout=subprocess.DEVNULL, stderr=proxy_err, cwd=ROOT)
        try:
            g6_counter_runner.wait_for_proxy(args.proxy_port, proxy)
            measured = command(180, args.proxy_port, "requests")
            (root / "command.json").write_text(json.dumps(measured, indent=2) + "\n", encoding="utf-8")
            if l1_runner.call(measured, root / "stdout.txt", root / "stderr.txt", 1200) != 0:
                raise ValueError(f"{label}/{workload}/c{concurrency} measured command failed")
            wait_for_trace(trace)
        finally:
            proxy.terminate()
            try:
                proxy.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proxy.kill()
                proxy.wait(timeout=10)


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
    versions = {"server_image": args.image, "model": args.model, "model_revision": args.revision,
                "candidate_binary_sha256": manifest_tool.sha256_file(args.candidate),
                "baseline_binary_sha256": manifest_tool.sha256_file(args.baseline),
                "manifest_sha256": manifest_tool.sha256_file(args.manifest)}
    failures = []
    try:
        manifest, config = manifest_tool.load_manifest(args.manifest, 'G7', args.candidate, args.baseline,
                                                       os.environ['CUDA_VISIBLE_DEVICES'])
        if (manifest['candidate_sha'] != args.sha or config['model'] != args.model or config['revision'] != args.revision
                or config['image'] != args.image):
            raise ValueError('G7 manifest identity differs from invocation')
        noise, noise_copy = {}, None
        if config.get('noise_floor'):
            floor = config['noise_floor']
            recorded = dt.datetime.fromisoformat(floor['recorded_utc'].replace('Z', '+00:00'))
            started = dt.datetime.fromisoformat(args.start_utc.replace('Z', '+00:00'))
            if recorded.tzinfo is None or started.tzinfo is None or recorded >= started:
                raise ValueError('G7 noise floor must predate the run')
            noise_copy = evidence / 'g7-noise-floor.json'
            shutil.copyfile(floor['path'], noise_copy)
            if manifest_tool.sha256_file(noise_copy) != floor['sha256']:
                raise ValueError('G7 noise floor changed before collection')
            noise = json.loads(noise_copy.read_text(encoding='utf-8'))
            if not isinstance(noise, dict): raise ValueError('G7 noise floor must be an object')
            versions['noise_floor_sha256'] = floor['sha256']
            versions['noise_floor_recorded_utc'] = floor['recorded_utc']
            versions['noise_floor_evidence_link'] = floor['evidence_link']
        for workload, concurrency in compare_small.POINTS:
            for label, binary in (("baseline", args.baseline), ("candidate", args.candidate)):
                try:
                    run_cell(args, config, label, binary, workload, concurrency)
                except (OSError, ValueError, KeyError, subprocess.SubprocessError) as exc:
                    failures.append(f"{label}/{workload}/c{concurrency}: {exc}")
        # Revalidate every frozen input after collection, not just the point metadata.
        manifest_tool.load_manifest(args.manifest, 'G7', args.candidate, args.baseline, os.environ['CUDA_VISIBLE_DEVICES'])
        if manifest_tool.sha256_file(args.manifest) != versions['manifest_sha256']:
            raise ValueError('G7 manifest changed during collection')
        if noise_copy is not None and manifest_tool.sha256_file(noise_copy) != versions['noise_floor_sha256']:
            raise ValueError('retained G7 noise floor changed during collection')
        observed = compare_small.compare(evidence / "baseline", evidence / "candidate",
                                         versions["baseline_binary_sha256"], versions["candidate_binary_sha256"], noise)
        observed["execution_failures"] = failures
        passed = observed["result"] == "PASS" and not failures
    except (OSError, ValueError, TypeError, KeyError, json.JSONDecodeError) as exc:
        observed, passed = {"error": str(exc), "execution_failures": failures}, False
    comparison = evidence / "g7-comparison.json"
    comparison.write_text(json.dumps(observed, indent=2, allow_nan=False) + "\n", encoding="utf-8")
    files = ["evidence/gpu.csv", "evidence/server.log", "evidence/binary-hashes.txt", "evidence/g7-comparison.json"]
    if (evidence / 'g7-noise-floor.json').is_file(): files.append('evidence/g7-noise-floor.json')
    files.extend(str(path.relative_to(args.out)) for label in ("baseline", "candidate")
                 for path in (evidence / label).rglob("*") if path.is_file())
    record = {"ID": "BC-PERF-003", "result": "PASS" if passed else "FAIL", "candidate_sha": args.sha,
              "attempt": 1, "start_utc": args.start_utc, "end_utc": dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z"),
              "owner": args.owner, "evidence_link": args.evidence_link, "gpu_class": gpu[0].strip(),
              "driver": gpu[1].strip() if len(gpu) > 1 else "", "versions": versions,
              "observed_values": observed, "thresholds": {"points": 6, "warmup_seconds": 30, "measurement_seconds": 180,
              "tolerance_pct": 5, "wider_floor": "previously frozen per point only", "nonzero_samples": True}, "evidence_paths": files}
    result_writer.write_record(args.out, record)
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
