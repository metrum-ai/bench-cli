<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Performance-number validation (issue #333)

Standard, versioned test that answers "did the performance numbers move?" for
`metrum-ai-bench-cli-llm` on a reference GPU, replacing ad-hoc validation.

## Pieces

- `spec.yaml` — the versioned pins: vLLM image digest, checkpoint revisions,
  serve flags, datasets and their SHA256, the point grid (W1/W2 × c1/c8/c32,
  30 s warmup + 180 s measurement), the request parameters, the gate
  tolerances, and the study reference numbers.
- `runner.sh` — runs the grid for one model on one GPU box, interleaving the
  baseline (release binary) and candidate (source build) per point, writing raw
  request logs, NDJSON, power CSVs and `point.json` per point. Idempotent: a
  point with `DONE` is skipped. `runner.sh --plan` is a dry run: it resolves
  the pins, prints the serve command and the full grid with binary/dataset
  checks, and exits 0 with no side effects — no docker, no GPU, no requests
  (CI exercises it on every change here).
- `compare.py` — derives per-point scalars from the raw logs and emits a
  per-point PASS/FAIL table (markdown + JSON). Gate: `system_output_tok_s` and
  `tpot_p50_ms` within tolerance (default 5%, or the point's noise floor if
  wider — `--noise-floor noise.json` with `{"<model>": {"W1": {"c1": {metric:
  pct}}}}`). TTFT is printed for information only (#313 cohort caveats).
  `--reference` adds informational deltas against a previous study. A point
  missing from either root is a FAIL verdict, listed in the table footer.
- `lint_spec.py` — structural spec check (required keys, runner-known model
  tags, digest/sha256 formats, grid and gate sanity, reference coverage);
  CI runs it on every change here, plus a negative self-test.
- `test_compare.py` — offline unit tests (stdlib `unittest`, synthetic
  exact-by-construction point roots): tolerance verdicts, noise-floor
  widening, missing points, study-reference columns. `python3 -m unittest
  test_compare.py`.
- `baselines/` — recorded results, one JSON per box/date, so drift over time
  is reviewable in git.

CI: `.github/workflows/perf-validation.yml` lints the spec, runs the unit
 tests, the plan mode and the compile checks on every change under
 `scripts/validation/perf/`. The GPU run itself is not CI: it is a scripted
 rental run wired into `docs/RELEASING.md` as a required pre-release step.

## Rules

- Self-hosted vLLM on our own rented/owned GPU only; loopback endpoint; the
  only credential on the box is the HF token in a 0600 file, never in argv.
- Record the dataset SHA256 actually used in the baseline file header; if it
  differs from the study freeze, say so (2026-10-26 Blackwell baseline: both
  datasets regenerated, hashes differ from the study freeze — see the header).
- Never put hostnames, IPs or GPU UUIDs in committed files or PR text.

## Freezing W1

The canonical W1 file is generated from the pinned CNN/DailyMail validation
split by `freeze_w1.py`. With Python 3.12.16, install
`huggingface_hub==2.3.0` and `pyarrow==26.0.0`, then run:

```sh
python3 freeze_w1.py --output W1.jsonl
```

The script checks the canonical SHA256 in `spec.yaml` before writing. This is
not the unpublished study generator: the 2026-10-26 baseline has a different
W1 hash, so historical W1 comparisons are not apples-to-apples. Use the same
frozen file for both builds of each same-box comparison.

## First baseline

`baselines/blackwell-se-2026-10-26.json` — RTX PRO 6000 Blackwell SE (96 GB),
driver 580.126.09, vLLM 0.32.0 pinned digest, v1.5.4 vs the
`integration-20261026` candidate, regenerated datasets W1 `b089f9fb…` /
W2 `e5480af1…`.
