<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Native cache metric semantics (development prototype)

This is **not complete profile support or live certification**. The initial
`cache-metric-profile.v1` Rust probe is not wired to CLI/report exports. Existing
scraper configuration and defaults remain unchanged. Tests are explicitly
synthetic, source-derived exposition, not captured live engine pages.

## Initial pinned source slices

| Slice | Source revision | Semantics |
| --- | --- | --- |
| vLLM APC | `aa4022dc8176db8f769f93445d845f8755f9b4ca` | Native prefix hit/query **token** counters; utilization gauge uses 1=100%. |
| vLLM external | same revision | Separate connector hit/query token counters, never silently summed with native counters. |
| SGLang | `188cf7da66938ca2e1ea9cbf7ebbf9a2c81edc2a` | Cached-token counter retains device/host/storage/total source labels; hit-rate gauge is not a phase counter. |
| LMCache | `34f8a409e3631eff51d0b6fff67ca9c68f123377` | Requested/hit/stored tokens differ from remote byte counters and retrieve-request counts. Stored tokens include evicted tokens: not resident occupancy. |
| Dynamo KVBM | `fa9852014950ed0aea74e56e1de441fc6de15307` | Directional transfer **blocks**, matched tokens and sliding-window hit-rate gauges are distinct. No block-to-byte conversion. |

Dynamo's pinned recipe explicitly requires **Dynamo <=1.6.x**; later releases
no longer ship KVBM. The legacy source slice is not a claim that current Dynamo
releases expose these metrics. No release aliases or TRT-LLM mappings have been
validated yet. Supplying a source revision is an **operator declaration**, not an
independent observation of the executable version or deployed tier.

## Conservative probe behavior

- Exact sample names; observed TYPE declarations are checked independently of
  the existing parser's `_total` suffix inference. Counter declaration spellings
  may differ between Prometheus and OpenMetrics without admitting new sample
  name aliases.
- Missing required families, unsupported/missing source revision, conflicting
  types, invalid values, duplicate identities and unsafe binary64 counter
  precision preserve Unknown, never an invented zero.
- Preserve each label set, including workers and source tiers; no replica or
  device/host/storage/total summation rule is implied. Bounds cover page size,
  series count and label dimensions/lengths. Raw observations deliberately do
  not implement Serialize: public export needs an explicit redacted projection.
- Gauge changes are not counter deltas. Reported window hit rates are not
  same-cohort phase hit rates. A known counter zero is different from a missing
  counter, and neither alone proves physical-cache absence or participation.

Remaining work includes exporter-derived coverage fixtures, explicit capability
and release-support metadata, public label policy, histogram suffix/unit
semantics, residency/eviction/failure families, TRT-LLM scope and opt-in
CLI/telemetry/report integration. Phase-scoped deltas and reset/identity handling
belong to the subsequent phase-delta feature. None is inferred by this prototype.
