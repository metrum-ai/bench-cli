#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Build and server-filter an LLM prompt pool.

Python 3.10+. Dependencies: requests. The script invokes
metrum-ai-bench-cli-prompts, compares /tokenize with usage.prompt_tokens once,
and keeps rows whose calibrated server ISL falls inside the requested window.
"""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path
from typing import Any

import requests


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--prompts-bin", default="metrum-ai-bench-cli-prompts")
    parser.add_argument("--dataset", default="metrum-ai/prompt-library")
    parser.add_argument("--config", default="full")
    parser.add_argument("--revision", required=True)
    parser.add_argument("--profile", default="chat-medium")
    parser.add_argument("--count", required=True, type=int)
    parser.add_argument("--seed", default=42, type=int)
    parser.add_argument("--url", required=True, help="OpenAI-compatible base URL ending in /v1")
    parser.add_argument("--model", required=True)
    parser.add_argument("--api-key", default="dummy")
    parser.add_argument("--isl-min", required=True, type=int)
    parser.add_argument("--isl-max", required=True, type=int)
    parser.add_argument("--calibration-tolerance", default=2, type=int)
    parser.add_argument("--work-dir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    return parser.parse_args()


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    with path.open(encoding="utf-8") as source:
        for line_no, line in enumerate(source, 1):
            if not line.strip():
                continue
            value = json.loads(line)
            if not isinstance(value, dict) or not isinstance(value.get("prompt"), str):
                raise ValueError(f"{path}:{line_no}: expected an object with string prompt")
            rows.append(value)
    return rows


def post_json(session: requests.Session, url: str, body: dict[str, Any]) -> dict[str, Any]:
    response = session.post(url, json=body, timeout=120)
    response.raise_for_status()
    value = response.json()
    if not isinstance(value, dict):
        raise ValueError(f"{url}: expected a JSON object")
    return value


def token_count(value: dict[str, Any]) -> int:
    count = value.get("count")
    if isinstance(count, int):
        return count
    tokens = value.get("tokens")
    if isinstance(tokens, list):
        return len(tokens)
    raise ValueError("/tokenize response has neither integer count nor token list")


def main() -> int:
    args = parse_args()
    if args.isl_min < 0 or args.isl_max < args.isl_min:
        raise ValueError("invalid ISL window")
    args.work_dir.mkdir(parents=True, exist_ok=True)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    mix = args.work_dir / "mix.jsonl"
    report = args.work_dir / "mix-report.json"

    subprocess.run(
        [
            args.prompts_bin,
            "--dataset", args.dataset,
            "--revision", args.revision,
            "--require-pinned-revision",
            "--config", args.config,
            "--count", str(args.count),
            "--seed", str(args.seed),
            "--profile", args.profile,
            "--output", str(mix),
            "--report", str(report),
        ],
        check=True,
    )

    rows = read_jsonl(mix)
    if not rows:
        raise ValueError("prompt extractor returned an empty mix")

    base = args.url.rstrip("/")
    headers = {"Authorization": f"Bearer {args.api_key}"}
    session = requests.Session()
    session.headers.update(headers)

    def messages(prompt: str) -> list[dict[str, str]]:
        return [{"role": "user", "content": prompt}]

    calibration_row = rows[0]
    calibration_messages = messages(calibration_row["prompt"])
    tokenize_value = post_json(
        session,
        f"{base}/tokenize",
        {"model": args.model, "messages": calibration_messages, "add_generation_prompt": True},
    )
    tokenize_tokens = token_count(tokenize_value)
    completion = post_json(
        session,
        f"{base}/chat/completions",
        {
            "model": args.model,
            "messages": calibration_messages,
            "max_tokens": 1,
            "temperature": 0,
        },
    )
    usage = completion.get("usage")
    prompt_tokens = usage.get("prompt_tokens") if isinstance(usage, dict) else None
    if not isinstance(prompt_tokens, int):
        raise ValueError("calibration response omitted usage.prompt_tokens")
    delta = prompt_tokens - tokenize_tokens
    if abs(delta) > args.calibration_tolerance:
        raise ValueError(
            f"/tokenize calibration differs from usage.prompt_tokens by {delta}; "
            f"allowed {args.calibration_tolerance}"
        )

    kept: list[dict[str, Any]] = []
    for row in rows:
        measured = token_count(
            post_json(
                session,
                f"{base}/tokenize",
                {"model": args.model, "messages": messages(row["prompt"]), "add_generation_prompt": True},
            )
        ) + delta
        if args.isl_min <= measured <= args.isl_max:
            enriched = dict(row)
            enriched["server_input_tokens"] = measured
            kept.append(enriched)

    with args.output.open("w", encoding="utf-8") as destination:
        for row in kept:
            destination.write(json.dumps(row, sort_keys=True) + "\n")

    calibration = {
        "model": args.model,
        "tokenize_tokens": tokenize_tokens,
        "usage_prompt_tokens": prompt_tokens,
        "delta": delta,
        "allowed_delta": args.calibration_tolerance,
        "input_rows": len(rows),
        "kept_rows": len(kept),
        "isl_window": [args.isl_min, args.isl_max],
    }
    (args.work_dir / "tokenize_calibration.json").write_text(
        json.dumps(calibration, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(json.dumps(calibration, sort_keys=True))
    return 0 if kept else 1


if __name__ == "__main__":
    raise SystemExit(main())
