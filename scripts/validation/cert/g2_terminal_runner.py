#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""G2 terminal matrix with transformed real frames and actual server interruption."""
import argparse
import datetime as dt
import json
import subprocess
from pathlib import Path

import g2_terminal_checks
import g3_runner
import manifest_tool
import r_checks
import r_runner
import result_writer


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
    cells, errors = {}, []
    for mode in ("short", "adequate", "remove_output", "remove_finish_and_done", "nonlength_finish", "duplicate_finish"):
        try:
            count, cap = 8, 4096 if mode == "adequate" else 128
            fault = None if mode in ("short", "adequate") else mode
            flags = ["--fail-on-error"] if fault in ("remove_output", "remove_finish_and_done", "nonlength_finish") else []
            data, trace, code, _ = r_runner.run_cell(args, f"g2-terminal-{mode}", args.candidate, count, cap, True, flags,
                                                      concurrency=1 if fault else 4, fault=fault)
            if mode == "short":
                cells[mode] = r_checks.truncation_cell(data, trace, count, False, code)
            elif mode == "adequate":
                cells[mode] = r_checks.adequate_cell(data, trace, count, code)
            else:
                cells[mode] = g2_terminal_checks.fault_cell(data, trace, code, mode, count)
        except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, subprocess.SubprocessError) as exc:
            cells[mode] = {"error": str(exc)}
            errors.append(mode)
    try:
        data, trace, code = g3_runner.run_cell(args, "error-first", count=8)
        rows = [json.loads(line) for line in data.splitlines() if line.strip()]
        requests = [r for r in rows if ".request." in r.get("schema_version", "") and r.get("phase") == "measure"]
        failures = [r for r in requests if r.get("error") is not None]
        summaries = [r for r in rows if ".summary." in r.get("schema_version", "")]
        if len(requests) != 8 or len(summaries) != 1 or summaries[0].get("attempted") != 8:
            raise ValueError("interruption cell must contain all eight measured attempts")
        if not failures or not any(e.get("kind") == "error" for e in g3_runner.events(evidence / "g3-error-first-trace.jsonl")):
            raise ValueError("real server kill produced no measured stream failure")
        cells["interruption"] = {"attempted": len(requests), "failed": len(failures), "exit_code": code}
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, subprocess.SubprocessError) as exc:
        cells["interruption"] = {"error": str(exc)}
        errors.append("interruption")
    files = ["evidence/" + name for name in ("gpu.csv", "server.log", "binary-hashes.txt", "container-id.txt")]
    files += [str(path.relative_to(args.out)) for pattern in ("g2-terminal-*", "g3-error-first-*")
              for path in evidence.glob(pattern) if path.is_file()]
    record = {"ID": "BC-GPU-004", "result": "FAIL" if errors else "PASS", "candidate_sha": args.sha, "attempt": 1,
              "start_utc": args.start_utc, "end_utc": dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z"),
              "owner": args.owner, "evidence_link": args.evidence_link, "gpu_class": gpu[0].strip(),
              "driver": gpu[1].strip() if len(gpu) > 1 else "",
              "versions": {"server_image": args.image, "model": args.model, "model_revision": args.revision,
                           "candidate_binary_sha256": manifest_tool.sha256_file(args.candidate),
                           "baseline_binary_sha256": manifest_tool.sha256_file(args.baseline),
                           "manifest_sha256": manifest_tool.sha256_file(args.manifest)},
              "observed_values": {"cells": cells, "failed_cells": errors},
              "thresholds": {"requests_per_cell": 8, "short_truncations_min": 1, "adequate_answers_min": 1,
                             "faults_required": ["remove_output", "remove_finish_and_done", "nonlength_finish", "duplicate_finish"],
                             "real_interruption_errors_min": 1}, "evidence_paths": files}
    result_writer.write_record(args.out, record)
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
