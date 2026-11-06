#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Record an unfinished case as FAIL when a lane exits prematurely."""
import argparse
import datetime as dt
from pathlib import Path
import result_writer


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--id", required=True)
    parser.add_argument("--sha", required=True)
    parser.add_argument("--start", required=True)
    parser.add_argument("--reason", required=True)
    args = parser.parse_args()
    result_writer.write_record(args.out, {"ID": args.id, "candidate_sha": args.sha, "result": "FAIL",
                                         "reason": args.reason, "attempt": 1, "start_utc": args.start,
                                         "end_utc": dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z"),
                                         "observed_values": {"request_count": None, "error_count": None}, "thresholds": {}, "evidence_paths": [],
                                         "gpu_class": None, "driver": None, "versions": {}})


if __name__ == "__main__":
    main()
