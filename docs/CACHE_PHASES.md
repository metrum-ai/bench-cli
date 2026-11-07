<!-- Copyright (c) 2026 Metrum AI, Inc. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Phase counter arithmetic (development prototype)

The library-only `cache-phase-delta.v1` unit consumes validated native probes.
It does not start requests, add a polling loop, integrate CLI/report exports or
claim complete phase validation. Runtime integration must extend the existing
#312 boundary machinery and #413 drain/reset/readiness/settling lifecycle.

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

Remaining work: captured source freshness/continuity/contamination evidence,
histogram family/transfer-window checks and existing-boundary runtime wiring.
These arithmetic APIs are not integrated CLI phase sampling or final exports.
Synthetic fixtures establish arithmetic contracts, not live exporter compatibility,
cache state, tier participation or performance.
