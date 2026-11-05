#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Run BC-REASON-006 with no max-token option or cap override."""
import argparse
import datetime as dt
import json
import subprocess
import sys
from pathlib import Path

import d_default_checks
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
    exit_code = None
    try:
        data, trace, exit_code, paths = r_runner.run_cell(args, "d-default", args.candidate, 64, None, True, [])
        files.extend(paths)
        stderr = (evidence / "d-default-stderr.txt").read_text(encoding="utf-8")
        observed = d_default_checks.evaluate(data, trace, stderr, exit_code)
        passed = True
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, subprocess.SubprocessError) as exc:
        observed, passed = {"error": str(exc), "exit_code": exit_code}, False
        files.extend("evidence/" + path.name for path in evidence.glob("d-default-*") if path.is_file() and "evidence/" + path.name not in files)
    record = {"ID": "BC-REASON-006", "result": "PASS" if passed else "FAIL", "candidate_sha": args.sha,
              "attempt": 1, "start_utc": args.start_utc, "end_utc": dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z"),
              "owner": args.owner, "evidence_link": args.evidence_link, "gpu_class": gpu[0].strip(),
              "driver": gpu[1].strip() if len(gpu) > 1 else "",
              "versions": {"server_image": args.image, "model": args.model, "model_revision": args.revision,
                           "candidate_binary_sha256": manifest_tool.sha256_file(args.candidate),
                           "baseline_binary_sha256": manifest_tool.sha256_file(args.baseline),
                           "manifest_sha256": manifest_tool.sha256_file(args.manifest)},
              "observed_values": observed,
              "thresholds": {"requests": 64, "max_token_override": False, "real_reasoning_min": 1,
                             "truncations_min": 1, "warning_contains_rate": True, "usage_coverage": 1.0},
              "evidence_paths": files}
    result_writer.write_record(args.out, record)
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
