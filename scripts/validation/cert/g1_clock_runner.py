#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Run BC-GPU-001 serial TTFT-mode cells against real Qwen streams."""
import argparse
import datetime as dt
import json
import subprocess
import sys
from pathlib import Path

import g1_clock_checks
import manifest_tool
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
    files = ["evidence/gpu.csv", "evidence/server.log", "evidence/binary-hashes.txt"]
    modes = (("default", []), ("byte", ["--infer-ttft-from-first-byte"]),
             ("fallback", ["--ttft-fallback", "first-token"]), ("allow", ["--allow-missing-ttft"]),
             ("fallback-allow", ["--ttft-fallback", "first-token", "--allow-missing-ttft"]))
    cells, errors = {}, []
    for mode, flags in modes:
        try:
            data, trace, code, paths = r_runner.run_cell(args, f"g1-clock-{mode}", args.candidate, 2, 4096, True, flags, concurrency=1)
            files.extend(paths)
            cells[mode] = g1_clock_checks.evaluate(data, trace, code)
        except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, subprocess.SubprocessError) as exc:
            cells[mode] = {"error": str(exc)}
            errors.append(mode)
    try:
        _, _, code, paths = r_runner.run_cell(args, "g1-clock-conflict", args.candidate, 2, 4096, True,
                                               ["--infer-ttft-from-first-byte", "--ttft-fallback", "first-token"], concurrency=1)
        files.extend(paths)
        stderr = (evidence / "g1-clock-conflict-stderr.txt").read_text(encoding="utf-8")
        if code == 0 or "--infer-ttft-from-first-byte" not in stderr or "--ttft-fallback" not in stderr:
            raise ValueError("conflicting options did not exit nonzero with diagnostic")
        cells["conflict"] = {"exit_code": code, "diagnostic_present": True}
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, subprocess.SubprocessError) as exc:
        cells["conflict"] = {"error": str(exc)}
        errors.append("conflict")
    files.extend("evidence/" + path.name for path in evidence.glob("g1-clock-*") if path.is_file() and "evidence/" + path.name not in files)
    record = {"ID": "BC-GPU-001", "result": "FAIL" if errors else "PASS", "candidate_sha": args.sha, "attempt": 1,
              "start_utc": args.start_utc, "end_utc": dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z"),
              "owner": args.owner, "evidence_link": args.evidence_link, "gpu_class": gpu[0].strip(),
              "driver": gpu[1].strip() if len(gpu) > 1 else "",
              "versions": {"server_image": args.image, "model": args.model, "model_revision": args.revision,
                           "candidate_binary_sha256": manifest_tool.sha256_file(args.candidate),
                           "baseline_binary_sha256": manifest_tool.sha256_file(args.baseline),
                           "manifest_sha256": manifest_tool.sha256_file(args.manifest)},
              "observed_values": {"cells": cells, "failed_modes": errors},
              "thresholds": {"reasoning_streams_per_valid_mode_min": 1, "visible_streams_per_valid_mode_min": 1,
                             "clock_error_seconds_max": 0.05, "conflict_exit_nonzero": True},
              "evidence_paths": files}
    result_writer.write_record(args.out, record)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
