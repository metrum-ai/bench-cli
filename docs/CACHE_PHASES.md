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
No source/tier/replica sum, ratio, rate, bandwidth, block-to-byte conversion or
quantile is produced by this first arithmetic slice.

Remaining work: explicit compatible denominators, zero/conflicting denominator
handling, validated non-overlapping worker weighting (not uncontrolled replica
sums), strict/optional coverage mode, histogram family/transfer-window checks,
source freshness/contamination evidence and existing-boundary runtime wiring.
Synthetic fixtures establish arithmetic contracts, not live exporter compatibility,
cache state, tier participation or performance.
