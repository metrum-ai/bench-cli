#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Gate every successful measured request against ISL and OSL windows."""

from __future__ import annotations

import argparse
import json
from collections import defaultdict
from pathlib import Path
from typing import Any


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--ndjson", required=True, type=Path)
    parser.add_argument("--isl-target", required=True, type=float)
    parser.add_argument("--isl-tolerance", required=True, type=float)
    parser.add_argument("--osl-target", required=True, type=float)
    parser.add_argument("--osl-tolerance", required=True, type=float)
    parser.add_argument("--out", default=Path("length_check.json"), type=Path)
    return parser.parse_args()


def request_lengths(row: dict[str, Any]) -> tuple[float, float] | None:
    # Tagged strategic/modality NDJSON request rows use input_tokens/output_tokens.
    if isinstance(row.get("input_tokens"), (int, float)) and isinstance(
        row.get("output_tokens"), (int, float)
    ):
        return float(row["input_tokens"]), float(row["output_tokens"])
    # Data-log request.v3 rows use prompt_tokens/completion_tokens. Server usage
    # wins; callers should only use tokenized_* when usage_missing is true.
    if row.get("usage_missing"):
        prompt = row.get("tokenized_prompt_tokens")
        completion = row.get("tokenized_completion_tokens")
    else:
        prompt = row.get("prompt_tokens")
        completion = row.get("completion_tokens")
    if isinstance(prompt, (int, float)) and isinstance(completion, (int, float)):
        return float(prompt), float(completion)
    return None


def main() -> int:
    args = parse_args()
    if args.isl_tolerance < 0 or args.osl_tolerance < 0:
        raise ValueError("tolerances must be non-negative")

    rows: list[dict[str, Any]] = []
    stages: dict[str, dict[str, int]] = defaultdict(
        lambda: {"compared": 0, "isl_out_of_window": 0, "osl_out_of_window": 0}
    )
    skipped = 0
    with args.ndjson.open(encoding="utf-8") as source:
        for line_no, line in enumerate(source, 1):
            if not line.strip():
                continue
            row = json.loads(line)
            if not isinstance(row, dict):
                continue
            kind = row.get("kind")
            schema = str(row.get("schema_version", ""))
            is_request = kind == "request" or schema.endswith("request.v3")
            if not is_request or row.get("warmup") is True:
                continue
            success = row.get("success")
            if success is False or row.get("error") not in (None, ""):
                continue
            pair = request_lengths(row)
            if pair is None:
                skipped += 1
                continue
            isl, osl = pair
            stage = str(row.get("stage", "measure"))
            stat = stages[stage]
            stat["compared"] += 1
            isl_bad = abs(isl - args.isl_target) > args.isl_tolerance
            osl_bad = abs(osl - args.osl_target) > args.osl_tolerance
            stat["isl_out_of_window"] += int(isl_bad)
            stat["osl_out_of_window"] += int(osl_bad)
            if isl_bad or osl_bad:
                rows.append(
                    {
                        "line": line_no,
                        "seq": row.get("seq"),
                        "stage": row.get("stage"),
                        "input_tokens": isl,
                        "output_tokens": osl,
                        "isl_ok": not isl_bad,
                        "osl_ok": not osl_bad,
                    }
                )

    total = sum(v["compared"] for v in stages.values())
    isl_bad = sum(v["isl_out_of_window"] for v in stages.values())
    osl_bad = sum(v["osl_out_of_window"] for v in stages.values())
    passed = total > 0 and skipped == 0 and isl_bad == 0 and osl_bad == 0
    result = {
        "pass": passed,
        "source": str(args.ndjson),
        "isl": {"target": args.isl_target, "tolerance": args.isl_tolerance},
        "osl": {"target": args.osl_target, "tolerance": args.osl_tolerance},
        "compared": total,
        "skipped_missing_lengths": skipped,
        "isl_out_of_window": isl_bad,
        "osl_out_of_window": osl_bad,
        "stages": dict(sorted(stages.items())),
        "mismatches": rows,
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(result, sort_keys=True))
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
