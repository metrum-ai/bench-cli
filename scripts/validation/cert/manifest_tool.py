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
    if lane in ("L1", "R", "D", "W", "G1", "G2", "G3", "G4", "G5", "G6", "G7") and config["model"] != "Qwen/Qwen3-8B":
        raise ValueError("this lane requires the pinned Qwen3-8B model")
    if lane in ("P1", "P2"):
        from perf_baseline import verify as verify_perf_baseline
        expected_model, expected_revision = {
            "P1": ("Qwen/Qwen3.8-27B", "1d4bf0f2ff6012fd82039f2fa52739d0dd7c60c0"),
            "P2": ("nvidia/Qwen3.8-27B-NVFP4", "482ca0f3832238542f8f5295dde86b5f22711d80"),
        }[lane]
        common = ["--max-model-len", "16384", "--max-num-seqs", "128", "--gpu-memory-utilization", "0.90",
                  "--default-chat-template-kwargs", '{"enable_thinking": false}']
        extra = ["--compilation-config", '{"cudagraph_mode":"PIECEWISE"}'] if lane == "P2" else []
        if (config["model"] != expected_model or config["revision"] != expected_revision or config["port"] != 8000
                or config["image"] != "vllm/vllm-openai@sha256:befccf0e7e4089aa23926bb6f887a85c0d46c2a99e6de5c5ffdf5cb7957c5b34"
                or flags != common + extra + ["--no-enable-prefix-caching"]):
            raise ValueError("27B server identity and flags must match the unchanged performance spec")
        archive = config.get("baseline_asset", {})
        if (not isinstance(archive, dict) or not isinstance(archive.get("path"), str)
                or not SHA.fullmatch(str(archive.get("sha256", ""))) or sha256_file(archive["path"]) != archive["sha256"]):
            raise ValueError("27B lanes require a pinned v1.7.2 release archive")
        verify_perf_baseline(archive["path"], baseline)
        if set(prompt_sets) != {"W1", "W2"}:
            raise ValueError("27B lanes require exactly W1 and W2")
        if (prompt_sets["W1"]["sha256"] not in ("4adc601afa3e4ce355a204dd042f1f64c3a24bdd7ef57a8a9c9ccd21d0ce0dd7",
                                                 "44340640d8bf5818cc9467f4e50b23abf3cf329005999b9c77b736d3fb3fa340")
                or prompt_sets["W2"]["sha256"] != "eb1ffdab20236c7aaadf63de3c03350076a47cccf796f279aacd551a38d8df8b"):
            raise ValueError("27B prompt sets must match the frozen spec")
        paths = [Path(prompt_sets[w]["path"]).resolve() for w in ("W1", "W2")]
        if paths[0].name != "W1.jsonl" or paths[1].name != "W2.jsonl" or paths[0].parent != paths[1].parent:
            raise ValueError("27B prompt sets must be W1.jsonl and W2.jsonl in one directory")
        for key in ("owner", "evidence_link"):
            if not isinstance(config.get(key), str) or not config[key].strip():
                raise ValueError(f"{lane} requires {key}")
        noise = config.get("noise_floor")
        if noise is not None and (not isinstance(noise, dict) or not isinstance(noise.get("path"), str)
                                  or not SHA.fullmatch(str(noise.get("sha256", ""))) or sha256_file(noise["path"]) != noise["sha256"]):
            raise ValueError("27B noise floor must be a pinned file")
        if lane == "P1":
            for key in ("all_smi_binary", "all_smi_protoc_crate", "telemetry_config"):
                item = config.get(key)
                if (not isinstance(item, dict) or not isinstance(item.get("path"), str)
                        or not SHA.fullmatch(str(item.get("sha256", ""))) or sha256_file(item["path"]) != item["sha256"]):
                    raise ValueError(f"P1 requires pinned {key}")
            if not os.access(config["all_smi_binary"]["path"], os.X_OK):
                raise ValueError("all-smi binary must be executable")
            if not isinstance(config.get("all_smi_checkout"), str) or not Path(config["all_smi_checkout"]).is_dir():
                raise ValueError("P1 requires the pinned all-smi source checkout")
            preset = Path(__file__).resolve().parents[3] / "docs/telemetry/examples/all-smi.yaml"
            if Path(config["telemetry_config"]["path"]).resolve() != preset.resolve():
                raise ValueError("P1 telemetry config must be the tracked loopback all-smi preset")
    if lane == "W":
        wrapper = config.get('wrapper', {})
        if (not isinstance(wrapper, dict) or not isinstance(wrapper.get('path'), str)
                or not SHA.fullmatch(str(wrapper.get('sha256', ''))) or not Path(wrapper['path']).is_file()
                or not os.access(wrapper['path'], os.X_OK) or sha256_file(wrapper['path']) != wrapper['sha256']
                or not isinstance(wrapper.get('version'), str) or not wrapper['version'].strip()):
            raise ValueError('W requires a pinned platform wrapper executable and version')
        command = config.get('wrapper_args')
        required = {'{candidate}', '{url}', '{prompt}', '{cli_log}', '{wrapper_log}', '{max_tokens}', '{count}', '{model}'}
        if not isinstance(command, list) or not all(isinstance(x, str) and x and '\x00' not in x for x in command) or not required <= set(command):
            raise ValueError('W wrapper args must include all standalone contract placeholders')
        if command.count('--streaming') != 1 or command.count('--extra-body-json') != 1:
            raise ValueError('W requires explicit forwarded streaming and thinking options')
        position = command.index('--extra-body-json')
        if position + 1 >= len(command) or json.loads(command[position + 1]) != {'chat_template_kwargs': {'enable_thinking': True}}:
            raise ValueError('W requires thinking ON')
        for key, flag in (('strict_args', '--strict-reasoning-truncation'), ('safety_args', '--ci-safety')):
            args = config.get(key)
            if not isinstance(args, list) or not all(isinstance(x, str) and x and '\x00' not in x for x in args) or args.count(flag) != 1:
                raise ValueError(f'W requires a forwarding argument array containing {flag}')
    if lane == "G6":
        strategic = config.get("strategic_binary")
        if not isinstance(strategic, dict) or not isinstance(strategic.get("path"), str) or not SHA.fullmatch(str(strategic.get("sha256", ""))):
            raise ValueError("G6 requires a pinned strategic binary")
        strategic_path = Path(strategic["path"]).resolve()
        if strategic_path != Path(candidate).resolve().parent / "metrum-ai-bench-cli-strategic" or not strategic_path.is_file() or not os.access(strategic_path, os.X_OK):
            raise ValueError("G6 strategic binary must be executable beside the candidate")
        if sha256_file(strategic_path) != strategic["sha256"]:
            raise ValueError("strategic binary hash mismatch")
    if lane == "G7":
        if not {"W1", "W2"} <= prompt_sets.keys():
            raise ValueError("G7 requires pinned W1 and W2 prompt sets")
        proxy_port = config.get("proxy_port")
        if not isinstance(proxy_port, int) or isinstance(proxy_port, bool) or not 1024 <= proxy_port <= 65535 or proxy_port == port:
            raise ValueError("G7 requires a distinct proxy port")
        if "--no-enable-prefix-caching" not in flags:
            raise ValueError("G7 requires disabled prefix caching")
        for key in ("owner", "evidence_link"):
            if not isinstance(config.get(key), str) or not config[key].strip():
                raise ValueError(f"G7 requires {key}")
        noise = config.get("noise_floor")
        if noise is not None:
            if (not isinstance(noise, dict) or not isinstance(noise.get("path"), str)
                    or not SHA.fullmatch(str(noise.get("sha256", ""))) or sha256_file(noise["path"]) != noise["sha256"]
                    or not isinstance(noise.get("recorded_utc"), str) or not noise["recorded_utc"].strip()
                    or not isinstance(noise.get("evidence_link"), str) or not noise["evidence_link"].strip()):
                raise ValueError("G7 noise floor requires a pinned file and prior independently reviewable evidence")
    if lane in ("L2", "L3", "L4"):
        modality, expected_model = {
            "L2": ("vlm", "Qwen/Qwen3-VL-8B-Instruct"),
            "L3": ("asr", "openai/whisper-large-v3-turbo"),
            "L4": ("imagegen", "Tongyi-MAI/Z-Image-Turbo"),
        }[lane]
        if config["model"] != expected_model or "default" not in prompt_sets:
            raise ValueError(f"{lane} requires its fixed sub-10B model and default input")
        for key in ("modality_binary", "sut"):
            item = config.get(key)
            if not isinstance(item, dict) or not isinstance(item.get("path"), str) or not SHA.fullmatch(str(item.get("sha256", ""))):
                raise ValueError(f"{lane} requires a pinned {key}")
            path = Path(item["path"]).resolve()
            if not path.is_file() or sha256_file(path) != item["sha256"]:
                raise ValueError(f"{lane} {key} hash mismatch")
            if key == "modality_binary" and (path != Path(candidate).resolve().parent / f"metrum-ai-bench-cli-{modality}" or not os.access(path, os.X_OK)):
                raise ValueError(f"{lane} binary must be executable beside the candidate")
        sut = json.loads(Path(config["sut"]["path"]).read_text(encoding="utf-8"))
        if sut.get("model", {}).get("id") != config["model"] or sut.get("model", {}).get("revision") != config["revision"]:
            raise ValueError(f"{lane} SUT model and revision must match")
        fixtures = {
            "L2": ("test-data/vlm/prompts.jsonl", "test-data/vlm/shapes-512.png"),
            "L3": ("test-data/asr/input.jsonl", "test-data/asr/truth.jsonl", "test-data/asr/1089-134686-0030.wav",
                   "test-data/asr/2961-961-0016.wav", "test-data/asr/6930-81414-0005.wav"),
            "L4": (),
        }[lane]
        pins = config.get("fixture_hashes", {})
        root = Path(__file__).resolve().parents[3]
        if not isinstance(pins, dict) or any(not SHA.fullmatch(str(pins.get(name, ""))) or sha256_file(root / name) != pins[name] for name in fixtures):
            raise ValueError(f"{lane} tracked fixture hashes missing or mismatched")
        if lane == "L2" and Path(prompt_sets["default"]["path"]).resolve() != (root / fixtures[0]).resolve():
            raise ValueError("L2 default prompt must be the tracked VLM fixture")
        if lane == "L3" and Path(prompt_sets["default"]["path"]).resolve() != (root / fixtures[0]).resolve():
            raise ValueError("L3 default input must be the tracked ASR fixture")
        for key in ("owner", "evidence_link"):
            if not isinstance(config.get(key), str) or not config[key].strip():
                raise ValueError(f"{lane} requires {key}")
    if lane == "V":
        if config["model"] != "Qwen/Qwen3-VL-8B-Thinking":
            raise ValueError("V requires the pinned thinking-capable Qwen3-VL-8B model")
        for key in ("vlm_binary", "vlm_image"):
            item = config.get(key)
            if not isinstance(item, dict) or not isinstance(item.get("path"), str) or not SHA.fullmatch(str(item.get("sha256", ""))):
                raise ValueError(f"V requires a pinned {key}")
            path = Path(item["path"]).resolve()
            if not path.is_file() or sha256_file(path) != item["sha256"]:
                raise ValueError(f"V {key} hash mismatch")
            if key == "vlm_binary" and (path != Path(candidate).resolve().parent / "metrum-ai-bench-cli-vlm" or not os.access(path, os.X_OK)):
                raise ValueError("V binary must be executable beside the candidate")
    if lane in ("R", "D", "V", "W", "G1", "G2", "G3", "G4", "G5", "G6"):
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
    parser.add_argument("--field", choices=["image", "model", "revision", "port", "proxy_port", "candidate_sha", "server_flags", "prompt_default", "prompt_W1", "prompt_W2", "sut_path", "strategic_path", "all_smi_binary_path", "telemetry_config_path", "owner", "evidence_link"], required=True)
    args = parser.parse_args()
    try:
        data, config = load_manifest(args.manifest, args.lane, args.candidate, args.baseline,
                                     os.environ.get("CUDA_VISIBLE_DEVICES"))
        if args.field == "candidate_sha":
            value = data[args.field]
        elif args.field == "prompt_default":
            value = config["prompt_sets"]["default"]["path"]
        elif args.field in ("prompt_W1", "prompt_W2"):
            value = config["prompt_sets"][args.field.removeprefix("prompt_")]["path"]
        elif args.field == "sut_path":
            value = config["sut"]["path"]
        elif args.field == "strategic_path":
            value = config["strategic_binary"]["path"]
        elif args.field == "all_smi_binary_path":
            value = config["all_smi_binary"]["path"]
        elif args.field == "telemetry_config_path":
            value = config["telemetry_config"]["path"]
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
