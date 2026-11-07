<!-- Copyright (c) 2026 Metrum AI, Inc. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Measured native metric windows and arithmetic (development)

`cache-phase-delta.v1` consumes validated native probes. Opt-in LLM and strategic
measured-window integration extends the existing synchronous telemetry boundary
machinery; it does not add another polling loop or certify physical cache state.
Final study exports and complete source/phase validation remain separate.

Private context binds the source, run epoch, declared source reference, stage,
phase and boundary edge to monotonic fetch windows. An after snapshot must
follow the before fetch; the baseline must finish before issuance, and terminal
fetch must start after drain/declared settling. These are caller-supplied
observations, not independently attested engine timestamps.

Per-series matching uses the complete private label set, exact metric and
profile/source revision. Missing boundaries, changed identity, invalid samples,
unsupported revision, changed/unavailable epoch or reversed/overlapping/late
windows yield null deltas plus reasons. A decrease never becomes zero. Known
unchanged cumulative counters remain observed zero differences; gauges retain
start/end observations but never become counter deltas. Individual cumulative
histogram components do not establish complete distributions or quantiles.

Continuity distinguishes observed markers, operator declarations and unknown.
An operator declaration is conditional evidence, not executable attestation.
Even a stable observed exporter marker cannot rule out every internal reset,
skipped manager snapshot or unrelated traffic. Reported endpoint differences
are not automatically attributable to this workload or a physical cache tier.

Public output excludes source keys, run IDs, epoch strings and raw labels.
Series IDs are scoped to a report; do not use them as stable worker identities.
The per-series slice produces no source/tier/replica sums, bandwidth,
block-to-byte conversion or quantiles.

The additional library `cache-phase-rate.v1` unit computes compatible reported
hit/query ratios over explicit, complete private worker selections. More than
one worker requires an operator-declared non-overlapping partition; labels are
never collapsed or inferred to be independent replicas. Unexpected or duplicate
partitions fail closed. Only matched native Hit/Query cumulative counters with
identical token/request units qualify; a gauge, miss counter or other denominator
is not silently substituted.

Weighting is sum(hit deltas)/sum(query deltas), not a mean of worker ratios.
Sums use exact integer arithmetic and public decimal strings even beyond the
binary64 integer range; the ratio itself is floating point. Decreases, missing
pairs, contradictory counters and zero denominator never become a valid zero
hit rate. Optional mode returns null plus reason; required coverage returns an
error carrying the diagnostic report. This is counter availability/compatibility
coverage, not proof of executable version, sample freshness or physical state.
Shared/unknown traffic yields Unknown. Explicit fixture or operator isolation
scopes remain labeled; declarations do not independently prove exclusivity.

## Opt-in measured windows

Both runners accept `--cache-phase-source NAME` selecting an exact source from
`--telemetry YAML`, with `--ndjson PATH` and the existing explicit
`--cache-metrics-url`, `--cache-metrics-profile` and optional source revision.
The selected configured URL must match the native endpoint; no source, revision,
worker or tier is autodetected. Defaults add no phase capture or analysis output.

The LLM native baseline replaces its legacy pre-measured boundary GET after all
warmup work drains. Its acknowledgement is awaited before measured issuance.
The terminal capture follows record drain, all request-task joins and optional
`--cache-phase-settle-ms` (0..10000 ms). Strategic uses separate stage-local
windows after warmup and reset readiness/settling, with terminal capture after
all stage tasks and priority writes drain. Per-request TTFT/E2E and record-derived
throughput windows exclude terminal settling; whole-run elapsed/setup can include
capture overhead. These windows are named
`measured`, not cold/warm physical states.

Native evidence and existing priority rows use the **same** boundary GET.
Native opted-in acquisition rejects redirects, credential-bearing URLs and
insecure TLS, bounds streaming bytes to the smaller of configured cap and 4 MiB,
requires strict UTF-8 and uses the smaller configured/source/5-second deadline.
Explicit telemetry authentication remains explicit; inference authentication is
not inherited. Legacy acquisition defaults are unchanged. Startup native
discovery and existing periodic telemetry remain distinct, not phase baselines.

`--cache-phase-epoch ID` is an **operator declaration**, not an observed serving
process or manager epoch. Without continuity evidence deltas remain Unknown.
`--cache-phase-required` additionally requires the exact supported source
reference and a continuity declaration; it fails on missing/invalid native
counter coverage, stopping measured issuance for failed baselines and returning
an error for failed terminal capture. This is numeric counter coverage, not
proof of executable version, sample age, workload exclusivity or physical state.
Optional coverage preserves null/reason instead of substituting zero. A native
semantic-limit failure does not fabricate a successful probe merely because
legacy HTTP/row coverage succeeded.

Versioned `native_cache_phase_analysis` appears in LLM summary modality metadata
and strategic config (one report per stage). Public analysis excludes raw labels,
endpoint URLs and continuity identifiers. Physical cache state, sample freshness
and workload exclusivity stay explicit Unknown; operator-declared deltas remain
conditional endpoint differences, not benchmark-attributed hit ratios. No worker
partition is inferred for aggregate hit rates. Core record versions and strategic
CSV column layout remain unchanged.

## Histogram component windows

`cache_histogram::histogram_windows` reuses the validated per-series endpoint
window. Complete private worker/other-label groups stay separate. A caller must
explicitly declare the exact bucket layout; it is never inferred from whichever
buckets happen to be present. The checker requires identical bucket identities,
matching count/sum components, cumulative baseline/end/delta buckets and an
infinite bucket equal to count. Missing/invalid components, changed epoch/layout,
contradictions or absent layout declarations produce Unknown and no complete
component result. Count and parent-unit sum remain distinct; no quantile,
transfer bandwidth, mean of worker distributions or tier sum is calculated.

This validates an **explicit declared layout and endpoint component window**,
not the full executable's histogram contract, sample freshness or manager
continuity. The CLI has no inferred layout declaration: observed histogram
groups therefore retain `layout_not_declared` and distribution validation remains
unverified. Even a successful library check does not independently attest the
operator's layout declaration or physical tier behavior.

Remaining work: captured source freshness/continuity/contamination evidence,
source-specific transfer attribution and final study exports.
Synthetic fixtures establish arithmetic contracts, not live exporter compatibility,
cache state, tier participation or performance.
