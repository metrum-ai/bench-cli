#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline preflight and direct-argv launch of individually frozen GPU cases."""
import argparse
import json
import os
import re
import subprocess
from pathlib import Path
from execution_evidence import digest, require_python


def pinned(item, executable=False):
    path = Path(item["path"])
    if not re.fullmatch(r"[a-f0-9]{64}", item.get("sha256", "")) or digest(path) != item["sha256"]:
        raise ValueError("pinned input hash mismatch")
    if executable and not os.access(path, os.X_OK):
        raise ValueError("pinned executable is not executable")
    return path.resolve()


def preflight(manifest):
    require_python()
    if not re.fullmatch(r"[a-f0-9]{40}", manifest.get("candidate_sha", "")):
        raise ValueError("candidate_sha must be a full commit")
    binary = pinned(manifest["candidate"], True)
    subprocess.run([str(binary), "--version"], check=True, capture_output=True, timeout=30)
    sut_path = pinned(manifest["sut"])
    sut = json.loads(sut_path.read_text())
    if not isinstance(sut.get("host_os"), str) or not sut["host_os"].strip():
        raise ValueError("SUT host_os is required")
    for memory in (sut.get("memory_gb"), sut.get("gpu", {}).get("memory_gb")):
        if type(memory) is not int or memory <= 0:
            raise ValueError("SUT system and GPU memory_gb must be positive integers")
    tokenizer = manifest["tokenizer"]
    if not re.fullmatch(r"[a-f0-9]{40}", tokenizer.get("revision", "")):
        raise ValueError("tokenizer revision must be pinned")
    pinned(tokenizer)
    pinned(manifest["all_smi"], True)
    for item in manifest.get("inputs", []):
        pinned(item)
    cases = manifest["cases"]
    if not cases:
        raise ValueError("no selected cases")
    for case, entry in cases.items():
        if not re.fullmatch(r"BC-[A-Z]+-[0-9]{3}", case):
            raise ValueError("invalid case ID")
        command = entry["command"]
        if not isinstance(command, list) or not command or not all(isinstance(s, str) and s and "\0" not in s for s in command):
            raise ValueError("case command must be an argv array")
        # Every executable is a separately pinned input, never an ambient PATH lookup.
        executable = pinned(entry["launcher"], True)
        if Path(command[0]).resolve() != executable:
            raise ValueError("case command does not match pinned launcher")
    return list(cases)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--preflight", action="store_true")
    parser.add_argument("--case", action="append")
    args = parser.parse_args()
    frozen = args.manifest.read_bytes()
    manifest = json.loads(frozen)
    cases = preflight(manifest)
    selected = args.case or cases
    if set(selected) - set(cases):
        raise ValueError("case not in frozen selection")
    if args.preflight:
        for case in selected:
            print(f"PREFLIGHT-OK {case}")
        return
    for case in selected:
        if args.manifest.read_bytes() != frozen:
            raise ValueError("manifest changed after preflight")
        # Recheck every pin between cases, not only on the initial launch.
        preflight(manifest)
        environment = {**os.environ, "CERT_INPUT_MANIFEST": str(args.manifest.resolve()),
                       "CERT_CASE_ID": case, "CERT_CANDIDATE": str(Path(manifest["candidate"]["path"]).resolve())}
        subprocess.run(manifest["cases"][case]["command"], check=True, env=environment)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, OSError, subprocess.SubprocessError, RuntimeError) as exc:
        raise SystemExit(f"case launch failed: {exc}") from exc
