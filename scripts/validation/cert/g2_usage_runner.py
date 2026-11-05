#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Run BC-GPU-006 with recorded original and selectively altered real SSE."""
import argparse
import datetime as dt
import json
import subprocess
import sys
from pathlib import Path

import g2_usage_checks
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
    cells, errors = {}, []
    for mode in ("control", "remove_usage", "remove_reasoning_split"):
        try:
            data, trace, code, paths = r_runner.run_cell(args, f"g2-usage-{mode}", args.candidate, 8, 4096, True, [],
                                                        concurrency=1, fault=None if mode == "control" else mode)
            files.extend(paths)
            cells[mode] = g2_usage_checks.evaluate(data, trace, code, mode, 8)
        except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, subprocess.SubprocessError) as exc:
            cells[mode] = {"error": str(exc)}
            errors.append(mode)
    files.extend("evidence/" + path.name for path in evidence.glob("g2-usage-*") if path.is_file() and "evidence/" + path.name not in files)
    record = {"ID": "BC-GPU-006", "result": "FAIL" if errors else "PASS", "candidate_sha": args.sha, "attempt": 1,
              "start_utc": args.start_utc, "end_utc": dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z"),
              "owner": args.owner, "evidence_link": args.evidence_link, "gpu_class": gpu[0].strip(),
              "driver": gpu[1].strip() if len(gpu) > 1 else "",
              "versions": {"server_image": args.image, "model": args.model, "model_revision": args.revision,
                           "candidate_binary_sha256": manifest_tool.sha256_file(args.candidate),
                           "baseline_binary_sha256": manifest_tool.sha256_file(args.baseline),
                           "manifest_sha256": manifest_tool.sha256_file(args.manifest)},
              "observed_values": {"cells": cells, "failed_modes": errors},
              "thresholds": {"requests_per_cell": 8, "control_usage_coverage": 1.0,
                             "missing_rates_null": True, "original_and_forwarded_streams_required": True},
              "evidence_paths": files}
    result_writer.write_record(args.out, record)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
