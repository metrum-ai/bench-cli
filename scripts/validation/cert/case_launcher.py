#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline preflight and direct-argv launch of individually frozen GPU cases."""
import argparse
import datetime as dt
import json
import os
import re
import subprocess
from pathlib import Path
import build_roles
from execution_evidence import digest, require_python, phase


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
    _, validation_binary = build_roles.check_manifest(manifest)
    import generated_inputs
    recipes = generated_inputs.structural_context(manifest)
    if recipes is not None:
        import manifest_tool
        baseline = pinned(manifest['baseline'], True)
        structural = {'structural_recipe_ids': set(recipes)}
        if 'generated_bindings' in manifest:
            import runtime_inputs
            runtime_inputs.verify(manifest)
            structural = {}  # Original concrete/domain checks remain mandatory.
        _, config = manifest_tool.load_manifest(manifest['lane_manifest']['path'], manifest['lane'],
            binary, baseline, '0', **structural)
        if any(recipe['tokenizer']['revision'] != config['revision'] for recipe in recipes.values()):
            raise ValueError('generator tokenizer revision must match lane model revision')
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
        if not isinstance(entry.get("out"), str) or not entry["out"]:
            raise ValueError("case requires an output directory")
        if command.count("--out") != 1 or command[command.index("--out") + 1:command.index("--out") + 2] != [entry["out"]]:
            raise ValueError("case command must use its frozen output directory")
        # Every executable is a separately pinned input, never an ambient PATH lookup.
        executable = pinned(entry["launcher"], True)
        if Path(command[0]).resolve() != executable:
            raise ValueError("case command does not match pinned launcher")
    from tokenizer_probe import probe
    probe(validation_binary, tokenizer['path'])
    if 'BC-GPU-010' in cases:
        import gpu010_builds
        gpu010_builds.bindings(manifest)
    return list(cases)


def run_case(manifest_path, manifest, case):
    build_roles.check_manifest(manifest)
    if case == 'BC-GPU-010':
        import gpu010_builds
        gpu010_builds.bindings(manifest)
    import signal
    import result_writer
    entry = manifest["cases"][case]
    out = Path(entry["out"]).resolve()
    if out.exists():
        raise FileExistsError("refusing to reuse an existing attempt directory")
    lifecycle = out.with_name(out.name + "-lifecycle")
    lifecycle.mkdir(parents=True, exist_ok=False)
    environment = {**os.environ, "CERT_INPUT_MANIFEST": str(Path(manifest_path).resolve()),
                   "CERT_CASE_ID": case, "CERT_PHASE_ROOT": str(lifecycle),
                   "CERT_CANDIDATE": str(Path(manifest["candidate"]["path"]).resolve())}
    started = dt.datetime.now(dt.timezone.utc).isoformat()
    process = None
    def interrupted(signum, _frame):
        raise InterruptedError(f"launcher interrupted by signal {signum}")
    previous = {sig: signal.signal(sig, interrupted) for sig in (signal.SIGINT, signal.SIGTERM)}
    try:
        if 'generated_inputs' in manifest:
            import runtime_inputs
            if 'generated_bindings' not in manifest:
                manifest_path, manifest = runtime_inputs.prepare(manifest_path, manifest, lifecycle / 'generated')
            preflight(manifest)  # Verify actual bytes and all original concrete criteria.
            entry = manifest['cases'][case]
            environment['CERT_INPUT_MANIFEST'] = str(Path(manifest_path).resolve())
        phase(lifecycle, case, 'inputs-ready')
        process = subprocess.Popen(entry["command"], env=environment, start_new_session=True)
        code = process.wait()
        if code:
            raise subprocess.CalledProcessError(code, entry["command"])
        target = out / "results" / (case + ".json")
        if not target.is_file():
            raise ValueError("case exited without an execution record")
        from execution_evidence import report_rows
        records = report_rows(out)
        if len(records) != 1 or records[0]["ID"] != case or records[0]["candidate_sha"] != manifest["candidate_sha"]:
            raise ValueError("case returned unexpected records")
        if records[0]["result"] != "PASS":
            raise ValueError("case failed; retained failure record is not certification")
        if case == 'BC-GPU-010':
            import gpu010_builds
            gpu010_builds.validate_record(manifest, records[0], out)
        phase(lifecycle, case, "record-validated")
    except (OSError, ValueError, subprocess.SubprocessError) as exc:
        if process is not None and process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=30)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
        out.mkdir(parents=True, exist_ok=True)
        target = out / "results" / (case + ".json")
        if not target.exists():
            kind = "infra_interrupted" if isinstance(exc, InterruptedError) or getattr(exc, "returncode", 0) in (-2, -15, 130, 143) else "infra_failed"
            # Failure provenance uses the same frozen pins as the child, without
            # retaining any unrelated environment values in the record.
            overrides = {key: environment[key] for key in ("CERT_INPUT_MANIFEST", "CERT_CASE_ID", "CERT_CANDIDATE")}
            before = {key: os.environ.get(key) for key in overrides}
            os.environ.update(overrides)
            try:
                result_writer.write_record(out, {
                    "ID": case, "candidate_sha": manifest["candidate_sha"], "result": "FAIL", "attempt": 1,
                    "start_utc": started, "end_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
                    "observed_values": {"request_count": None, "error_count": None}, "thresholds": {},
                    "evidence_paths": [], "gpu_class": None, "driver": None, "versions": {},
                    "reason": str(exc), "infra_history": [{"type": kind, "reason": str(exc)}]})
            finally:
                for key, value in before.items():
                    if value is None:
                        os.environ.pop(key, None)
                    else:
                        os.environ[key] = value
        else:
            # A failing launcher/cleanup cannot be masked by a previously written PASS.
            with (out / "results" / "lane-error.json").open("x") as stream:
                json.dump({"error": str(exc)}, stream)
        raise
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)


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
            import generated_inputs
            print(f"{generated_inputs.preflight_label(manifest)} {case}")
        return
    for case in selected:
        if args.manifest.read_bytes() != frozen:
            raise ValueError("manifest changed after preflight")
        # Recheck every pin between cases, not only on the initial launch.
        preflight(manifest)
        run_case(args.manifest, manifest, case)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, OSError, subprocess.SubprocessError, RuntimeError) as exc:
        raise SystemExit(f"case launch failed: {exc}") from exc
