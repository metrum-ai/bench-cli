#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Perf-validation spec lint (issue #333). Structural check of spec.yaml so a
# bad pin or a stale point grid fails CI instead of a GPU run: every required
# key present, model tags match the runner's, digests and hashes well-formed,
# the point grid and gates sane, and the study reference covers the full grid.
#
# Usage:
#   lint_spec.py [spec.yaml]       # exit 0 = valid, exit 1 = errors printed

import re
import sys

import yaml

MODEL_TAGS = ("q38-27b", "q38-27b-nvfp4")
HEX64 = re.compile(r"^[0-9a-f]{64}$")
IMAGE_DIGEST = re.compile(r"^[\w./-]+@sha256:[0-9a-f]{64}$")


class SpecError(Exception):
    pass


def require(mapping, key, where):
    if key not in mapping:
        raise SpecError(f"{where}: missing required key '{key}'")
    return mapping[key]


def lint(path):
    errors = []
    try:
        spec = yaml.safe_load(open(path))
    except yaml.YAMLError as e:
        return [f"{path}: invalid YAML: {e}"]
    if not isinstance(spec, dict):
        return [f"{path}: not a YAML mapping"]

    def check(fn, *args):
        try:
            fn(*args)
        except SpecError as e:
            errors.append(str(e))
        except (TypeError, ValueError, KeyError, IndexError) as e:
            errors.append(f"{path}: malformed value: {e}")

    def sut(s):
        image = require(s, "vllm_image", "sut")
        if not IMAGE_DIGEST.match(str(image)):
            raise SpecError(f"sut.vllm_image: not a pinned digest: {image}")
        for key in ("gpu", "driver", "vllm_version", "serve_common_flags"):
            require(s, key, "sut")
        if not isinstance(s["serve_common_flags"], list) or not s["serve_common_flags"]:
            raise SpecError("sut.serve_common_flags: must be a non-empty list")

    def models(m):
        if not m:
            raise SpecError("models: empty")
        for tag, entry in m.items():
            if tag not in MODEL_TAGS:
                raise SpecError(f"models: unknown tag {tag!r}; runner.sh handles {MODEL_TAGS}")
            where = f"models.{tag}"
            require(entry, "checkpoint", where)
            rev = require(entry, "revision", where)
            if not re.fullmatch(r"[0-9a-f]{40}", str(rev)):
                raise SpecError(f"{where}.revision: not a 40-hex git sha: {rev}")
            require(entry, "extra_serve_flags", where)

    def datasets(d):
        if not d:
            raise SpecError("datasets: empty")
        for w, entry in d.items():
            where = f"datasets.{w}"
            for key in ("max_tokens", "source", "generation", "study_sha256"):
                require(entry, key, where)
            if not isinstance(entry["max_tokens"], int) or entry["max_tokens"] <= 0:
                raise SpecError(f"{where}.max_tokens: must be a positive integer")
            if not HEX64.match(str(entry["study_sha256"])):
                raise SpecError(f"{where}.study_sha256: not a sha256 hex digest")
            if w == "W1" and not HEX64.match(str(require(entry, "canonical_sha256", where))):
                raise SpecError(f"{where}.canonical_sha256: not a sha256 hex digest")

    def points(p):
        for key in ("workloads", "concurrency", "warmup_seconds", "measure_seconds"):
            require(p, key, "points")
        wl = p["workloads"]
        cc = p["concurrency"]
        if not isinstance(wl, list) or not all(isinstance(x, str) for x in wl):
            raise SpecError("points.workloads: must be a list of names")
        if not isinstance(cc, list) or not all(isinstance(x, int) and x > 0 for x in cc):
            raise SpecError("points.concurrency: must be positive integers")
        for key in ("warmup_seconds", "measure_seconds"):
            if not isinstance(p[key], int) or p[key] <= 0:
                raise SpecError(f"points.{key}: must be a positive integer")
        req = require(p, "request", "points")
        for key in ("mode", "streaming", "temperature", "seed", "api_key"):
            require(req, key, f"points.request")
        if not req["streaming"]:
            raise SpecError("points.request.streaming: the spec pins streaming runs")

    def builds(b):
        require(b, "baseline", "builds")
        require(b, "candidate", "builds")
        require(b, "order", "builds")

    def gates(g):
        reg = require(g, "regression", "gates")
        metrics = require(reg, "metrics", "gates.regression")
        if set(metrics) != {"system_output_tok_s", "tpot_p50_ms"}:
            raise SpecError(f"gates.regression.metrics: must be exactly the two gate metrics, got {metrics}")
        tol = require(reg, "tolerance_pct", "gates.regression")
        if not isinstance(tol, (int, float)) or not 0 < tol <= 50:
            raise SpecError("gates.regression.tolerance_pct: must be in (0, 50]")
        require(reg, "ttft", "gates.regression")
        require(g, "drift", "gates")

    def reference(r, wl, cc):
        for tag in MODEL_TAGS:
            if tag not in r:
                raise SpecError(f"reference: missing model {tag}")
            for w in wl:
                if w not in r[tag]:
                    raise SpecError(f"reference.{tag}: missing workload {w}")
                for c in cc:
                    if f"c{c}" not in r[tag][w]:
                        raise SpecError(f"reference.{tag}.{w}: missing concurrency c{c}")
                    for m in ("tok_s", "tpot_p50_ms"):
                        if not isinstance(r[tag][w][f"c{c}"].get(m), (int, float)):
                            raise SpecError(f"reference.{tag}.{w}.c{c}: missing numeric {m}")

    try:
        require(spec, "spec_version", "spec")
        if spec["spec_version"] != 1:
            raise SpecError(f"spec.spec_version: unsupported (want 1): {spec['spec_version']}")
        require(spec, "name", "spec")
    except SpecError as e:
        errors.append(str(e))

    for fn, arg in [
        (sut, require(spec, "sut", "spec") if "sut" in spec else None),
        (models, spec.get("models")),
        (datasets, spec.get("datasets")),
        (points, spec.get("points")),
        (builds, spec.get("builds")),
        (gates, spec.get("gates")),
    ]:
        if arg is None:
            continue
        check(fn, arg)

    if isinstance(spec.get("points"), dict):
        p = spec["points"]
        wl = p.get("workloads") or []
        cc = p.get("concurrency") or []
        ref = spec.get("reference")
        if ref is not None:
            check(reference, ref, wl, cc)

    return errors


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else "scripts/validation/perf/spec.yaml"
    errors = lint(path)
    if errors:
        for e in errors:
            print(f"spec lint: {e}", file=sys.stderr)
        sys.exit(1)
    print("spec lint: ok")


if __name__ == "__main__":
    main()
