<!-- Copyright (c) 2026 Metrum AI. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->
# Endpoints comparison evidence: 2026-10-23

Metrum AI study, finalized 2026-10-24 UTC. Exploratory same-SUT research, not an accepted MLPerf submission.

## Verdict

Decode TPOT agrees within 5% for all 80 averaged online cross-tool point comparisons; TTFT does not reliably agree. Dense Qwen3.8 W2/c8 TTFT is arrival-pattern confounded: singleton-send medians are 229.81 versus 228.18 ms, while replacement-cohort mixtures differ. No TTFT parser bug or hidden-token correction is demonstrated. See the timing/cohort audit before interpreting differences.

All 224 canonical points report zero request-error rate. All five rented instances are API-verified gone; final ledger spend is $49.63. Native AIME evidence is unavailable after the dense BF16 timeout and aggregate-only for dense NVFP4 where per-item outputs were not retained; neither limitation is represented as zero accuracy.

## Evidence and traceability

- [Full report](https://github.com/metrum-ai/bench-cli/releases/download/evidence-mlperf-endpoints-20261023/SUMMARY.md); [ZIP bundle](https://github.com/metrum-ai/bench-cli/releases/download/evidence-mlperf-endpoints-20261023/evidence-mlperf-endpoints-20261023.zip); [SHA256 file](https://github.com/metrum-ai/bench-cli/releases/download/evidence-mlperf-endpoints-20261023/evidence-mlperf-endpoints-20261023.zip.sha256sum).
- ZIP SHA256: `c314b082fbadbf52a16cec8c1edc495e54ae503cdbb1679c1565e93675b9fcf6` (rebuilt 2026-10-25 for the erratum below; original `01dd4cce4ba1b501e7dbcf496c70d3d6b221bedbae19363b7648d320460bffa6`).
- Bundle contains scalar reports, 224-row points-final.csv, exact run/extract/telemetry helpers, dataset hashes and unmodified bench summary.v3 rows with SUT/configuration/client environment plus native reference summaries.
- Local per-request logs back the scalar observations but are not published. No prompts, generated responses, GPQA content, exporter labels, public rental addresses, box hostnames, GPU UUIDs or secret values are included. Loopback endpoints and generic study/provider-default paths are deliberately retained in unmodified summaries.
- MANIFEST.json hashes every member except itself. The publication audit reports counts by candidate category and no blocked locations.

## Methodology and versions

The study measured v1.5.4 and main@30b8872 (full SHA below), pinned at study start. v1.5.5 was released mid-study; it was not measured. This documentation branch is rebased onto current main without changing those measured version pins.

| Component | Pin / settings |
|---|---|
| MLPerf reference | `93ec321e9868abc48757b12f4fcdca92f3ff20b3`, unmodified, frozen uv dependencies |
| Bench main | `30b887295927d8e0cacdfe55b8e949f7fe6e5640`, unmodified locked release build |
| Bench release | v1.5.4; archive SHA256 `185c0fd9f7b8f6f18d1c7a383866d8d5a24940cdc23a6f618093bb63d5fec36a`; per-asset checksum verified |
| all-smi fork | `3b6eaec3364d3c2eb897bf1433ee93eef609c6a9` |
| Server | Official vLLM 0.32.0, image digest and exact flags in summary.json SUT inventory |
| Hardware | One RTX PRO 6000 Blackwell Server Edition 96 GB per lane, Shadeform/massedcompute, driver 580.126.09, CUDA 13.0 |
| Workload | W1 CNN/DailyMail validation, 512 prompts, max 128; W2 prompt-library full/code-medium, 512 prompts, max 512; seed 42 |
| Request | Streamed chat, temperature 0, seed 42, thinking disabled; client local to GPU |
| Online | c1/8/32/64/128; separate30s warm-server process plus drain, then180s issuance plus drain; reference/release/main/reference-repeat/release-repeat |
| Offline | Native reference max_throughput; bench c128 saturation proxy, not identical semantics; fixed 512 requests except the three points in the erratum below |
| Cache/precision | Prefix caching disabled for canonical points. Dense NVFP4 uses PIECEWISE graphs; MoE NVIDIA checkpoint is mixed FP8 attention/W4A16 weight-only Marlin, not native W4A4 |

Repeated-run noise is point/metric-specific. Differences count beyond-noise only when exceeding both 5% and the repeat range; main uses release repeat noise as a proxy because it has one scheduled pass. Small sample tails, output framing/tokenization, native duration/drain and percentile conventions remain explicit caveats. Power is measured GPU board power, not official provisioned whole-system power.

## Release c128 comparison (averaged online passes)

| Model tag | Workload | Output tok/s | TPOT p50 ms | Board tok/J | Knee |
|---|---|---:|---:|---:|---:|
| q38-27b | W1 | 600.553 | 203.007 | 1.077 | 8 |
| q38-27b | W2 | 1175.523 | 102.888 | 2.474 | 8 |
| q36-35b | W1 | 1798.804 | 68.741 | 4.175 | 64 |
| q36-35b | W2 | 2202.786 | 56.602 | 5.979 | 64 |
| q38-27b-nvfp4 | W1 | 1076.445 | 115.895 | 2.114 | 32 |
| q38-27b-nvfp4 | W2 | 1892.633 | 63.363 | 4.448 | 64 |
| q36-35b-nvfp4 | W1 | 2617.023 | 47.979 | 5.791 | 128 |
| q36-35b-nvfp4 | W2 | 3812.661 | 32.949 | 9.354 | 128 |

## Quality and standards boundary

summary.json evals records full promptfoo suites with correct/n, Wilson 95% intervals and errors; strict variants reuse primary responses. Native accuracy cross-check is AIME-only: its preset contains 15 distinct problems repeated twice. First-attempt deduplication, partial/unavailable correctness and differing prompt/extraction rules are explicit in AIME-RECONCILE.md. Do not compare all 30 native attempts with 30 distinct promptfoo problems as the same population.

The upstream submission-checker PointSummary parser cross-checks units/rate arithmetic for native reference output and a clearly marked bench scalar adapter. This is not full compliance validation. Custom pools, duration/accuracy/coverage/power disclosures and distinct metric definitions prevent claiming a valid Endpoints submission.

## Follow-ups

Approved bench issues: [#303](https://github.com/metrum-ai/bench-cli/issues/303), [#304](https://github.com/metrum-ai/bench-cli/issues/304), [#305](https://github.com/metrum-ai/bench-cli/issues/305), [#306](https://github.com/metrum-ai/bench-cli/issues/306), [#307](https://github.com/metrum-ai/bench-cli/issues/307), [#308](https://github.com/metrum-ai/bench-cli/issues/308), [#309](https://github.com/metrum-ai/bench-cli/issues/309), [#310](https://github.com/metrum-ai/bench-cli/issues/310), [#311](https://github.com/metrum-ai/bench-cli/issues/311), [#312](https://github.com/metrum-ai/bench-cli/issues/312), [#313](https://github.com/metrum-ai/bench-cli/issues/313). Upstream Endpoints and all-smi findings are report-only; no issues were filed there.

This PR commits scalar docs only. The evidence bundle is a pre-release targeted at the tested main commit, latest=false; its tag does not match v* release/deploy triggers.

## Erratum 2026-10-25

`summary.json` (and the release's `points-final.csv`) now carry a per-point `comparability_note`. No measured values changed and no points were removed.

- **q36-35b W1 bench-rel Offline (2,600 requests): not like-for-like. Exclude it from Offline comparisons.** It is one complete run with a single run id, not an appended retry. It ran before the harness fix at 2026-10-23T21:22Z that limited Offline points to 512 requests, so it kept sending for the full 180 s.
- **q38-27b W2 bench-rel and bench-src Offline (504 and 502 requests):** both runs hit the 180 s sending limit before issuing all 512 requests. The requests in flight then completed, and none failed.
