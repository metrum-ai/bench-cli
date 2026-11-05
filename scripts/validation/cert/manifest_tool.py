#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Validate frozen lane pins and emit server arguments without shell evaluation."""
import argparse
import hashlib
import json
import os
import re
import sys
from pathlib import Path

SHA = re.compile(r"[0-9a-f]{64}\Z")
REV = re.compile(r"[0-9a-f]{40}\Z")
IMAGE = re.compile(r"[a-zA-Z0-9._/:+-]+@sha256:[0-9a-f]{64}\Z")
MODEL = re.compile(r"[a-zA-Z0-9][a-zA-Z0-9_.-]*/[a-zA-Z0-9][a-zA-Z0-9_.-]*\Z")
LANE = re.compile(r"(?:P[12]|L[1-4]|R|D|V|W|G[1-7])\Z")
GPU = re.compile(r"(?:[0-9]+|GPU-[0-9a-fA-F-]+)\Z")
FORBIDDEN = {"--host", "--port", "--model", "--revision", "--tokenizer", "--tokenizer-revision"}


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate manifest key: {key}")
        result[key] = value
    return result


def sha256_file(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_manifest(path, lane, candidate, baseline, gpu):
    if not LANE.fullmatch(lane):
        raise ValueError("invalid lane")
    if not isinstance(gpu, str) or not GPU.fullmatch(gpu):
        raise ValueError("CUDA_VISIBLE_DEVICES must identify exactly one GPU")
    with Path(path).open(encoding="utf-8") as stream:
        data = json.load(stream, object_pairs_hook=unique_object)
    if not isinstance(data, dict) or not REV.fullmatch(str(data.get("candidate_sha", ""))):
        raise ValueError("manifest must pin the full candidate commit")
    for kind, path_value in (("candidate", candidate), ("baseline", baseline)):
        binary = Path(path_value)
        expected = data.get(f"{kind}_binary_sha256")
        if not binary.is_file() or not os.access(binary, os.X_OK):
            raise ValueError(f"{kind} binary is missing or not executable")
        if not isinstance(expected, str) or not SHA.fullmatch(expected) or sha256_file(binary) != expected:
            raise ValueError(f"{kind} binary hash mismatch")
    lanes = data.get("lanes")
    if not isinstance(lanes, dict) or not isinstance(lanes.get(lane), dict):
        raise ValueError("lane server configuration missing")
    config = lanes[lane]
    if not isinstance(config.get("image"), str) or not IMAGE.fullmatch(config["image"]):
        raise ValueError("server image must be pinned by sha256 digest")
    if not isinstance(config.get("model"), str) or not MODEL.fullmatch(config["model"]):
        raise ValueError("model must be an explicit repository identifier")
    if not isinstance(config.get("revision"), str) or not REV.fullmatch(config["revision"]):
        raise ValueError("model revision must be a full commit SHA")
    port = config.get("port")
    if not isinstance(port, int) or isinstance(port, bool) or not 1024 <= port <= 65535:
        raise ValueError("lane port must be in 1024..65535")
    flags = config.get("server_flags", [])
    if not isinstance(flags, list) or not all(isinstance(flag, str) and flag and "\x00" not in flag for flag in flags):
        raise ValueError("server_flags must be a string argument array")
    if any(flag.split("=", 1)[0] in FORBIDDEN for flag in flags):
        raise ValueError("server flags cannot override pinned identity or port")
    prompt_sets = config.get("prompt_sets")
    if not isinstance(prompt_sets, dict) or not prompt_sets:
        raise ValueError("at least one pinned prompt set is required")
    for prompt in prompt_sets.values():
        if not isinstance(prompt, dict) or not isinstance(prompt.get("path"), str):
            raise ValueError("prompt entry requires a path and SHA256")
        expected = prompt.get("sha256")
        if not isinstance(expected, str) or not SHA.fullmatch(expected) or sha256_file(prompt["path"]) != expected:
            raise ValueError("prompt set hash mismatch")
    if lane in ("L1", "R", "G6") and config["model"] != "Qwen/Qwen3-8B":
        raise ValueError("this lane requires the pinned Qwen3-8B model")
    if lane == "G6":
        strategic = config.get("strategic_binary")
        if not isinstance(strategic, dict) or not isinstance(strategic.get("path"), str) or not SHA.fullmatch(str(strategic.get("sha256", ""))):
            raise ValueError("G6 requires a pinned strategic binary")
        strategic_path = Path(strategic["path"]).resolve()
        if strategic_path != Path(candidate).resolve().parent / "metrum-ai-bench-cli-strategic" or not strategic_path.is_file() or not os.access(strategic_path, os.X_OK):
            raise ValueError("G6 strategic binary must be executable beside the candidate")
        if sha256_file(strategic_path) != strategic["sha256"]:
            raise ValueError("strategic binary hash mismatch")
    if lane in ("R", "G6"):
        if "default" not in prompt_sets:
            raise ValueError(f"{lane} requires a default prompt set")
        proxy_port = config.get("proxy_port")
        if not isinstance(proxy_port, int) or isinstance(proxy_port, bool) or not 1024 <= proxy_port <= 65535 or proxy_port == port:
            raise ValueError(f"{lane} requires a distinct proxy port")
        if "--no-enable-prefix-caching" not in flags or not any(
                flags[index:index + 2] == ["--reasoning-parser", "qwen3"] or flags[index] == "--reasoning-parser=qwen3"
                for index in range(len(flags))):
            raise ValueError(f"{lane} requires qwen3 reasoning parser and disabled prefix caching")
        for key in ("owner", "evidence_link"):
            if not isinstance(config.get(key), str) or not config[key].strip():
                raise ValueError(f"{lane} requires {key}")
    if lane == "L1":
        if "default" not in prompt_sets:
            raise ValueError("L1 requires a default prompt set")
        candidate_path = Path(candidate).resolve()
        if candidate_path.name != "metrum-ai-bench-cli-llm":
            raise ValueError("L1 candidate must be named metrum-ai-bench-cli-llm")
        helper = config.get("prompt_binary")
        if not isinstance(helper, dict) or not isinstance(helper.get("path"), str) or not SHA.fullmatch(str(helper.get("sha256", ""))):
            raise ValueError("L1 requires a pinned prompt helper")
        helper_path = Path(helper["path"]).resolve()
        if helper_path != candidate_path.parent / "metrum-ai-bench-cli-prompts" or not os.access(helper_path, os.X_OK):
            raise ValueError("prompt helper must be executable beside the candidate")
        if sha256_file(helper_path) != helper["sha256"]:
            raise ValueError("prompt helper hash mismatch")
        sut = config.get("sut")
        if not isinstance(sut, dict) or not isinstance(sut.get("path"), str) or not SHA.fullmatch(str(sut.get("sha256", ""))):
            raise ValueError("L1 requires a pinned SUT file")
        if sha256_file(sut["path"]) != sut["sha256"]:
            raise ValueError("SUT file hash mismatch")
        sut_data = json.loads(Path(sut["path"]).read_text(encoding="utf-8"))
        if not isinstance(sut_data, dict) or not isinstance(sut_data.get("model"), dict):
            raise ValueError("SUT model metadata missing")
        if sut_data["model"].get("id") != config["model"] or sut_data["model"].get("revision") != config["revision"]:
            raise ValueError("SUT model or revision does not match the server")
        for key in ("owner", "evidence_link"):
            if not isinstance(config.get(key), str) or not config[key].strip():
                raise ValueError(f"L1 requires {key}")
    return data, config


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--lane", required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--field", choices=["image", "model", "revision", "port", "proxy_port", "candidate_sha", "server_flags", "prompt_default", "sut_path", "strategic_path", "owner", "evidence_link"], required=True)
    args = parser.parse_args()
    try:
        data, config = load_manifest(args.manifest, args.lane, args.candidate, args.baseline,
                                     os.environ.get("CUDA_VISIBLE_DEVICES"))
        if args.field == "candidate_sha":
            value = data[args.field]
        elif args.field == "prompt_default":
            value = config["prompt_sets"]["default"]["path"]
        elif args.field == "sut_path":
            value = config["sut"]["path"]
        elif args.field == "strategic_path":
            value = config["strategic_binary"]["path"]
        else:
            value = config.get(args.field, [])
        if isinstance(value, list):
            for part in value:
                sys.stdout.buffer.write(part.encode("utf-8") + b"\0")
        else:
            print(value)
        return 0
    except (OSError, ValueError, TypeError, KeyError, json.JSONDecodeError) as exc:
        print(f"manifest validation failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
