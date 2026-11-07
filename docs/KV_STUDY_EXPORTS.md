<!-- Copyright (c) 2026 Metrum AI, Inc. All rights reserved. -->
# Observed KV study exports

The opt-in `kv-study-export.v1` envelope binds a declaration conforming to
[kv-study-report.v1](../schemas/kv-study-report.v1.json) to actual supplied runtime
rows. Its [output schema](../schemas/kv-study-export.v1.json) is separate from
existing request, summary and strategic CSV versions. Existing output formats
and defaults do not change when these flags are absent.

## Scope and safe defaults

This is an **engineering, non-certifying** report. The manifest is not an engine
attestation. Runtime model/tokenizer/template/connector/version/layout/routing,
flags, dataset rights and tier declarations are not silently upgraded to observed
facts. Dataset validation references retain `kv-dataset-manifest.v1`; this does not
create a second rights schema.

Every emitted physical `cache_state` remains `unknown`. A seed claiming observed
residency/external state, a verified reset, or observed tier evidence is refused
before traffic. A logical arm named `cold_populate` is not verified cold. Request
cached-token zero is not a physical reset, and a nonzero report does not identify
a tier. [Cached-token cohorts](CACHED_TOKENS.md), [native profiles](CACHE_METRIC_PROFILES.md)
and [measured windows](CACHE_PHASES.md) retain their independent evidence scopes.

## CLI

Both LLM and strategic runners accept the paired flags:

- `--kv-study-manifest PATH`: a local, schema-valid declaration, at most 4 MiB.
- `--kv-study-prefix PREFIX`: reserve new `PREFIX.study.json`, `.study.csv` and
  `.study.html` before traffic; never overwrite existing files. Parent directories
  must exist. Output aliases with primary artifacts are refused. Failed partial
  writes/reservations owned by the export plan are cleaned up.

The manifest needs exactly one logical **measured** phase per planned stage, in
stage order (one for LLM). Duplicate IDs, unsupported warmup-phase bindings,
missing stages and cache-policy/effective-order mismatches fail before traffic.
Warmup/drain observations remain excluded from measured tables; generic warmup is
never called cache-neutral. Interrupted or incomplete stages report observed rows,
not scheduled request counts, and do not establish completion or physical state.
A reset failure remains a failed run; reset execution is the existing trusted
[stage lifecycle](STRATEGIC_BENCHMARKING.md#trusted-stage-reset-lifecycle-v190-development),
not a new execution path in this exporter.

For an authorized mock fixture, a declaration with `cache_policy: reuse`,
`order: preserve` and one logical measured phase can be used as follows:

```sh
metrum-ai-bench-cli-llm --url "$MOCK_CHAT_URL" --api-key dummy \
  --scenario kv-study --mode chat --model mock --prompts prompts.jsonl \
  --num-requests 20 --concurrency 1 --warmup-requests 0 --max-tokens 8 \
  --infer-ttft-from-first-byte --cache-policy reuse --prompt-order preserve \
  --kv-study-manifest study-seed.json --kv-study-prefix result
```

Use only authorized endpoints; this example is not permission to run a live
engine or a performance validation. Source-derived and synthetic fixtures certify
fixture behavior only.

A seed must include all required fields, even when null. For example, build one
without inventing unknown runtime facts:

```python
import json
runtime = dict.fromkeys([
    "model_revision", "tokenizer_revision", "chat_template_sha256", "engine",
    "engine_version", "connector", "connector_version", "flags_sha256",
    "block_size_tokens", "kv_dtype", "kv_layout", "tensor_parallelism", "routing"
])
runtime["tiers"] = []
seed = {
    "schema_version": "kv-study-report.v1", "study_id": "mock-study",
    "reuse_kind": "exact_prefix", "runtime": runtime, "datasets": [],
    "workload": {
        "cache_policy": "reuse", "order": "preserve",
        "effective_order_sha256": None, "body_fingerprints_sha256": None,
        "history_mode": "recorded", "arrival_mode": "closed_loop",
        "concurrency": 1, "input_distribution": None, "output_distribution": None,
        "warmup_isolation": "unverified"
    },
    "phases": [{
        "id": "populate", "kind": "cold_populate", "cache_state": "unknown",
        "reset": {
            "scope": [], "acknowledged": False, "ready": False, "settled": False,
            "status": "not_requested", "reason": None, "started_unix_ns": None,
            "acknowledged_unix_ns": None, "ready_unix_ns": None, "settled_unix_ns": None
        },
        "requests": 0, "errors": 0, "cached_token_known": 0, "cached_token_unknown": 0
    }],
    "limitations": ["Synthetic declaration; no physical or deployed-engine validation"]
}
with open("study-seed.json", "w") as output:
    json.dump(seed, output)
```

The exporter replaces phase counts with actual measured observations, not the
seed's proposed counts. Other manifest metadata remains explicitly unverified;
observed configuration/producer evidence is separately scoped in
`runtime_bindings`. Final-body hashes and policy effects come from existing
request records, not re-rendering or a second nonce implementation. Invalid
binding/effect data is Unknown/null and does not become zero. Attempt evidence
is not proof of server receipt or that all planned requests were issued.

## JSON / CSV / HTML reconciliation

All views use the same canonical phase model. CSV contains one phase row, not one
row per timing source: `cohorts_json`, `scheduled_delays_json`, `native_window_json`,
`observed_final_bodies_json`, `runtime_bindings_json` and `run_comparisons_json`
preserve nested units, populations, exclusions and provenance. Report-level
bindings/comparisons repeat as metadata cells; do not sum those repeated cells.
Phase IDs are report-local; labels have an inert `phase: ` prefix, and HTML
escapes all user-controlled text.

TTFT tables retain headline clock/source separately from visible-answer TTFT,
including missing values and small-n warnings. Percentiles are type 7; worker,
turn or cohort percentiles are never averaged. Empty samples remain null/Unknown.
Native window evidence retains the existing conditional continuity and coverage
reasons, with no inferred physical state, histogram quantiles or tier sums.

Scheduling tables include client queue delay and scheduled-arrival latency
(queue delay + request latency), in seconds, only with scheduling evidence and
finite nonnegative values. Missing legacy offsets do not become observed zero.
Errors/invalid timings are excluded with coverage; delay alone does not prove
backlog/cap engagement, which remains Unknown.

## Repetitions and conditional uncertainty

Reuse the existing dispatcher's `metrum-ai-bench-cli llm --runs N ...` facility.
For study exports with `N > 1`, output prefixes become `PREFIX.run-0001`,
`PREFIX.run-0002`, etc.; single-run and non-study behavior stays unchanged. The
existing data-log cross-run throughput statistics remain throughput statistics,
**not paired TTFT intervals**. Each child still requires new output files.

The Rust report API provides `paired_run_mean_difference` and `add_comparison`.
Supply explicitly matched baseline/treatment exports and phase indices per run
pair, an explicit clock, a seed and a SHA-256 declaration of fixed controls and
independent complete run pairs (including matched collection windows). Run
completion/independence is not independently attested by these record adapters.
That declaration is **operator-declared, unverified**;
it is not a reset, deployment or physical-state attestation.

Only complete matched clock populations enter the calculation. Measured turn and
prefix-group sample counts must match **within** each pair, ignoring cache-hit
cohort (the intended treatment response). Cluster sizes may differ **across** run
pairs; that must not increase a large run's bootstrap weight. Errors, invalid
or missing clock observations, reused arms/clusters, duplicate pair IDs, mixed
reuse kinds, missing declarations and fewer than two run pairs produce Unknown.
Run means use sample-count weights across source-compatible turn/cohort groups;
then existing `stats::bootstrap_mean_ci` resamples **whole paired run mean
differences**, not correlated turns. The statistic is treatment minus baseline
mean TTFT in seconds, not a p95 interval or a ratio. Small-cluster warnings remain.
Single-run uncertainty stays Unknown; no confidence interval is fabricated.

## Methodology and publication checklist

- Use paired independent trusted-reset runs. Randomize or interleave logical
  arm order across repetitions; record the actual schedule and seed. Do not
  present one monotonically ordered sweep as an independent paired experiment.
- Keep workload, source revisions, canonical order, mutation policy, output
  targets/distributions, model, routing, batching and chunked-prefill settings
  fixed, or disclose them as factors. Separate declared output targets from
  generated output and observed group sizes from corpus estimates.
- Use held-out prefix-disjoint warmup or a trusted reset after complete warmup
  drain/readiness/settling. A generic warmup is not cache-neutral. Consume the
  existing renderer/overlap/reset evidence; do not replace its implementation.
- Compare cold/populate, resident and pressured re-hit arms separately. Unknown
  state cannot be published as cold; reported reuse alone cannot prove external
  participation or physical tier bandwidth/latency.
- Under open-loop load disclose scheduled-arrival latency, client delay,
  scheduling coverage and cap/backlog engagement. Do not hide coordinated
  omission behind token-only TTFT improvements.
- Preserve all errors, unknown counters, missing timing samples and small-n
  warnings. Use whole-run/session clusters, not correlated turns, for intervals.
- Exact-prefix, approximate chunk reuse and compression are different studies.
  Cache token reports are not saved-prefill, quality, codec or selective-recompute
  evidence. Include those measurements before making their respective claims.
- Read `publication_blockers`: declarations and numeric coverage do not remove
  physical/freshness/independence limitations. This output is non-certifying and
  makes no standards/performance threshold claim.

Offline tests cover a synthetic cold miss → resident hit → eviction → external
hit → external miss/recompute state machine, actual HTTP/CSV record binding,
failed reset refusal, invalid/Unknown evidence and cross-export reconciliation.
They are not live exporter captures or executable/physical-tier certification.
