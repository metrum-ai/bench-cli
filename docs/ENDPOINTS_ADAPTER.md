<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Endpoints-compatible, explicitly non-certifying artifact adapter (#306)

`metrum-ai-bench-cli endpoints-export` converts a **completed** bench run data
log (`--data-log` JSONL with request and summary rows) into an
Endpoints-shaped artifact bundle:

```
<out>/
  point.yaml                      # tool identity/version, run_id, model,
                                  # workload, scenario intent + disclosure,
                                  # units, provenance
  system_desc.json                # declared-only SUT subset (GPU, driver,
                                  # runtime, model, host OS, cost)
  system_power.json               # operator-declared power descriptors with
                                  # source sha256, or {declared: false}
  performance/result_summary.json # upstream-parser-compatible keys
  config.yaml                     # scenario identity + load pattern (never an
                                  # upstream version string)
  accuracy/NOT_PROVIDED           # marker; accuracy is never fabricated
  crosscheck.json                 # record-derived vs summary-derived
                                  # arithmetic, per metric
  NONCOMPLIANCE.md                # which submission checks remain impossible
```

The bundle is **parseable by the upstream submission-checker's Endpoints
result parser** (`tools/submission/submission_checker/parsers/endpoints_parser.py`
in mlcommons/inference; integration test
`endpoints_export_upstream_parser_smoke` runs it when
`MLCOMMONS_INFERENCE_CHECKOUT` points at a checkout). That is **parse
compatibility, not compliance**: the bundle is never an MLPerf or Endpoints
submission. The study's pinned checker commit
(`fda43ef88fffcef4e9da81df2b71dbb355dd78e8`) is not publicly fetchable, so
the PointSummary cross-check from the study's `CROSSCHECK.json` is an
operator-supplied step; the adapter's own crosscheck.json cross-verifies
record-derived vs summary-derived arithmetic instead, and flags mismatches.

## Usage

```bash
metrum-ai-bench-cli endpoints-export \
  --data-log run.jsonl \
  --out-dir bundle/ \
  --workload W1 \
  --scenario server \
  --sut sut.json            # optional; declares system_desc.json content
  --power-desc power.json   # optional; declared power descriptors only
```

The data log must contain exactly one complete (non-partial) run summary with
matching request rows; partial-only, ambiguous multi-run, or summary-less logs
are hard errors — the adapter never manufactures a point from a broken run.

## Scenario mapping disclosures

`--scenario server` is the native shape for paced/closed-loop bench runs.
`--scenario offline`, `loadstair`, and `pareto` are exported **with an
explicit disclosure** in `point.yaml`/`config.yaml`: bench has no LoadStair or
Pareto arrival pattern, and no native Offline burst mode (see
[#309](https://github.com/metrum-ai/bench-cli/issues/309)); a closed-loop or
fixed-rate run labeled Offline is a proxy, not the same arrival process.

## Definition flags

Native bench definitions are preserved, never silently transformed:

- **TTFT** is first **visible content token** (not first byte) — flagged
  per metric in `result_summary.json` (`bench_definition`) and `point.yaml`.
- **TPOT** is a per-request `(latency − ttft) / (completion_tokens − 1)`
  percentile — flagged the same way; it is not a steady-state aggregate.
- **Token rates** are emitted only when every successful row reports server
  usage (`usage_missing` rows drop the rate instead of undercounting it).

## Power

`system_power.json` records operator-declared descriptors (from
`--power-desc`, passed through with the file's sha256) or
`declared: false`. PDU, whole-system and provisioned-power facts are never
synthesized.
