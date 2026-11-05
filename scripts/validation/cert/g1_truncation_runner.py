#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""G1 real-server truncation classification matrix (BC-GPU-002)."""
import argparse
import datetime as dt
import json
import subprocess
import sys
from pathlib import Path

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
    files = ["evidence/gpu.csv", "evidence/server.log", "evidence/binary-hashes.txt"]
    cells = {}
    error = None
    for mode, flags, strict in (("default", [], False), ("strict", ["--strict-reasoning-truncation"], True),
                                ("fail-on-error", ["--fail-on-error"], False), ("ci", ["--ci-safety"], True)):
        try:
            data, trace, code, paths = r_runner.run_cell(args, f"g1-{mode}", args.candidate, 64, 128, True, flags)
            files.extend(paths)
            cells[mode] = r_checks.truncation_cell(data, trace, 64, strict, code)
        except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, subprocess.SubprocessError) as exc:
            cells[mode] = {"error": str(exc)}
            error = "one or more truncation modes failed"
    if error:
        files.extend("evidence/" + path.name for path in evidence.glob("g1-*") if path.is_file() and "evidence/" + path.name not in files)
    record = {"ID": "BC-GPU-002", "result": "FAIL" if error else "PASS", "candidate_sha": args.sha, "attempt": 1,
              "start_utc": args.start_utc, "end_utc": dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z"),
              "owner": args.owner, "evidence_link": args.evidence_link, "gpu_class": gpu[0].strip(),
              "driver": gpu[1].strip() if len(gpu) > 1 else "",
              "versions": {"server_image": args.image, "model": args.model, "model_revision": args.revision,
                           "candidate_binary_sha256": manifest_tool.sha256_file(args.candidate),
                           "baseline_binary_sha256": manifest_tool.sha256_file(args.baseline),
                           "manifest_sha256": manifest_tool.sha256_file(args.manifest)},
              "observed_values": {"cells": cells, "error": error},
              "thresholds": {"requests_per_mode": 64, "real_truncations_per_mode_min": 1,
                             "default_and_fail_on_error_exit": 0, "strict_and_ci_exit_nonzero": True},
              "evidence_paths": files}
    result_writer.write_record(args.out, record)
    return 1 if error else 0


if __name__ == "__main__":
    sys.exit(main())
