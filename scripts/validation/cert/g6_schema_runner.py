#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Exercise real reasoning schema exports plus frozen migration fixtures."""
import argparse
import datetime as dt
import json
import subprocess
import sys
from pathlib import Path

import l1_runner
import manifest_tool
import r_checks
import r_runner
import result_writer
import schema_checks

ROOT = Path(__file__).resolve().parents[3]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("out", "candidate", "strategic", "manifest", "prompt"):
        parser.add_argument("--" + name, type=Path, required=True)
    for name in ("sha", "model", "revision", "image", "start-utc", "owner", "evidence-link"):
        parser.add_argument("--" + name, required=True)
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--proxy-port", type=int, required=True)
    args = parser.parse_args()
    evidence = args.out / "evidence"
    gpu_line = (evidence / "gpu.csv").read_text(encoding="utf-8").strip().split(",", 1)
    paths = ["evidence/gpu.csv", "evidence/server.log", "evidence/binary-hashes.txt"]
    prefix = "g6-schema-"
    try:
        fixture_out, fixture_err = evidence / (prefix + "fixtures.txt"), evidence / (prefix + "fixtures-error.txt")
        fixture_command = ["cargo", "test", "--locked", "--offline", "--test", "schema_history", "--test", "schema_migration"]
        (evidence / (prefix + "fixture-command.json")).write_text(json.dumps(fixture_command, indent=2) + "\n", encoding="utf-8")
        if l1_runner.call(fixture_command, fixture_out, fixture_err, 1800) != 0:
            raise ValueError("frozen old-reader/wrapper migration tests failed")
        llm_ndjson = evidence / (prefix + "llm-telemetry.jsonl")
        llm_data, llm_trace, code, _files = r_runner.run_cell(args, prefix + "llm", args.candidate, 16, 4096, True,
                                                               ["--ndjson", str(llm_ndjson)])
        r_checks.adequate_cell(llm_data, llm_trace, 16, code)
        strategic_out, strategic_err = evidence / (prefix + "strategic.json"), evidence / (prefix + "strategic-error.txt")
        strategic_csv, strategic_ndjson = evidence / (prefix + "strategic.csv"), evidence / (prefix + "strategic-telemetry.jsonl")
        strategic_html = evidence / (prefix + "strategic.html")
        command = [str(args.strategic), "--url", f"http://localhost:{args.port}/v1/chat/completions", "--api-key", "dummy",
                   "--model", args.model, "--kind", "chat", "--streaming", "--requests-per-stage", "8", "--sweep", "1",
                   "--prompts", str(args.prompt), "--max-tokens", "4096", "--temperature", "0", "--seed", "42",
                   "--extra-body-json", json.dumps({"chat_template_kwargs": {"enable_thinking": True}}),
                   "--timeout-seconds", "1200", "--csv", str(strategic_csv), "--html", str(strategic_html),
                   "--ndjson", str(strategic_ndjson)]
        (evidence / (prefix + "strategic-command.json")).write_text(json.dumps(command, indent=2) + "\n", encoding="utf-8")
        if l1_runner.call(command, strategic_out, strategic_err, 1800) != 0:
            raise ValueError("strategic live run exited nonzero")
        observed = schema_checks.evaluate(llm_data, llm_ndjson.read_text(encoding="utf-8"), llm_trace,
                                          strategic_out.read_text(encoding="utf-8"), strategic_csv.read_text(encoding="utf-8"),
                                          strategic_ndjson.read_text(encoding="utf-8"))
        observed["migration_fixtures_exit"] = 0
        passed = True
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, subprocess.SubprocessError) as exc:
        observed, passed = {"error": str(exc)}, False
    paths.extend("evidence/" + path.name for path in evidence.glob(prefix + "*") if path.is_file())
    record = {"ID": "BC-GPU-016", "result": "PASS" if passed else "FAIL", "candidate_sha": args.sha, "attempt": 1,
              "start_utc": args.start_utc, "end_utc": dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z"),
              "owner": args.owner, "evidence_link": args.evidence_link, "gpu_class": gpu_line[0].strip(),
              "driver": gpu_line[1].strip() if len(gpu_line) > 1 else "",
              "versions": {"server_image": args.image, "model": args.model, "model_revision": args.revision,
                           "candidate_binary_sha256": manifest_tool.sha256_file(args.candidate),
                           "strategic_binary_sha256": manifest_tool.sha256_file(args.strategic),
                           "manifest_sha256": manifest_tool.sha256_file(args.manifest)},
              "observed_values": observed,
              "thresholds": {"live_reasoning_streams_min": 1, "live_answers_min": 1, "migration_fixture_exit": 0,
                             "current_schema_exports_valid": True, "old_reader_rejects_new_version": True},
              "evidence_paths": paths}
    result_writer.write_record(args.out, record)
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
