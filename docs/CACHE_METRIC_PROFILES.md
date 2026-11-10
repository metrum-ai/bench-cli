<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Native cache metric semantics (development prototype)

This is **not complete profile support or live certification**. The initial
`cache-metric-profile.v1` Rust probe has opt-in startup discovery plumbing in both
chat runners, with a separate `cache-metric-capture.v1` public projection. This
is not phase-delta or physical-cache certification. Existing scraper
configuration and defaults remain unchanged. Tests are explicitly
synthetic, source-derived exposition, not captured live engine pages.

## Initial pinned source slices

| Slice | Source revision | Semantics |
| --- | --- | --- |
| vLLM APC | `aa4022dc8176db8f769f93445d845f8755f9b4ca` | Native prefix hit/query **token** counters; utilization gauge uses 1=100%. |
| vLLM external | same revision | Separate connector hit/query token counters, never silently summed with native counters. |
| vLLM offload | same revision | Flat directional byte counters, cumulative transfer time in **seconds** and allocation-failure **events**. Deprecated transfer-type aggregates are deliberately excluded, not summed or aliased. |
| vLLM 4ba APC, external, offload | `4ba787ec9959e566d3b587a796dff07673324da5` | Separate exact-revision slices of the same metric families and units. See the revision-specific caveats below; these profiles do not imply tier attribution. |
| SGLang v0.5.23 | `297e54fd105e974cdf401e5b4d20253a22c16f90` | Separate exact-revision slice; see [source and certification notes](KV_SGLANG_CERTIFICATION.md). Same cached-token source labels and hit-rate gauge semantics, never occupancy. |
| SGLang | `188cf7da66938ca2e1ea9cbf7ebbf9a2c81edc2a` | Cached-token counter retains device/host/storage/total source labels; hit-rate gauge is not a phase counter. |
| LMCache v0.5.7 MP | `236f3eec32a2d330662a1795d90a01a1e0782c49` | Separate MP lookup-token and store/load chunk slice; no legacy aliases or resident occupancy. See [source and certification notes](KV_LMCACHE_CERTIFICATION.md). |
| LMCache | `34f8a409e3631eff51d0b6fff67ca9c68f123377` | Requested/hit/stored tokens differ from remote byte counters and retrieve-request counts. Stored tokens include evicted tokens: not resident occupancy. Separate reported cache/storage byte gauges, eviction-operation events, evicted keys and failed-eviction **candidate counts** retain their units. A failed selection can count up to 64 candidates, not one failed call. |
| Dynamo KVBM | `fa9852014950ed0aea74e56e1de441fc6de15307` | Directional transfer **blocks**, matched tokens and sliding-window hit-rate gauges are distinct. Object read/write failure counters also count **blocks**, not requests. No block-to-byte conversion. |
| TRT-LLM collector | `a3febfbfd0324bded62007ffd7d5241a06cbfc00` | Cached prompt tokens, manager/iteration reused or missed blocks, accepted connector loads, reported directional bytes and layout/utilization gauges remain separate. Not Triton backend `nv_trt_llm_*` aliases. |
| TRT-LLM stable 1.2.1 | `376f7e1bd8ed543f75014309e3fd4b237e9b0e73` | Separate `trt-llm121` profile: completed requests and four duration histograms at `/prometheus/metrics`; cached tokens Unknown, reason `not exported by 1.2.1`. [Source and runtime scope](KV_R5_TRTLLM.md). |

`Sglang0523` is a separate `ProfileId` with its own exact source revision.
It reuses the `SGLANG` metric spec because the pinned-source review and
[upstream exporter serialization fixture](../tests/fixtures/sglang0523-source-slice.prom)
confirm identical cached-token counter and hit-rate gauge definitions.
This shared metric spec does not create a source or release alias.

Dynamo's pinned recipe explicitly requires **Dynamo <=1.6.x**; later releases
no longer ship KVBM. The legacy source slice is not a claim that current Dynamo
releases expose these metrics. No release aliases have been validated. Supplying a source revision is an **operator declaration**, not an
independent observation of the executable version or deployed tier.

## vLLM source revision 4ba787ec

The following source references are pinned to commit `4ba787ec9959e566d3b587a796dff07673324da5`, the source revision reported by the v0.33.0 image's configuration. An image label is a publisher claim, not independent build provenance.

- [`loggers.py`](https://github.com/vllm-project/vllm/blob/4ba787ec9959e566d3b587a796dff07673324da5/vllm/v1/metrics/loggers.py) declares local and connector prefix queries and hits in tokens and increments them from separate scheduler statistics. [`kv_cache_manager.py`](https://github.com/vllm-project/vllm/blob/4ba787ec9959e566d3b587a796dff07673324da5/vllm/v1/core/kv_cache_manager.py) records local queries and hits for eligible lookups. [`stats.py`](https://github.com/vllm-project/vllm/blob/4ba787ec9959e566d3b587a796dff07673324da5/vllm/v1/metrics/stats.py) keeps preempted request statistics separate from the counters used by the logger. The utilization gauge reports the engine's cache usage ratio; it is not physical allocation proof.
- [`scheduler.py`](https://github.com/vllm-project/vllm/blob/4ba787ec9959e566d3b587a796dff07673324da5/vllm/v1/core/sched/scheduler.py) counts local lookup statistics at admission and connector queries/hits from its planned external matches. The connector counts are admission/acceptance observations, not independent evidence of a completed transfer or a specific physical medium. Cache-read opt-out and scheduling behavior differ from the older revision, so counts need not be comparable across versions or workloads.
- [`offloading/metrics.py`](https://github.com/vllm-project/vllm/blob/4ba787ec9959e566d3b587a796dff07673324da5/vllm/distributed/kv_transfer/kv_connector/v1/offloading/metrics.py) defines flat directional load/store byte and time counters, synchronous lookup duration and failed store-allocation events. [`offloading/worker.py`](https://github.com/vllm-project/vllm/blob/4ba787ec9959e566d3b587a796dff07673324da5/vllm/distributed/kv_transfer/kv_connector/v1/offloading/worker.py) records completed successful transfer size/time only when both are supplied; a failed job asserts. These aggregates neither identify a request nor establish a physical CPU-tier hit. The corresponding definition and logger files are byte-identical to the older profile revision, but worker failure handling, cache-lookup opt-outs and CPU-manager store-threshold accounting differ. Deprecated transfer-type aggregates are not accepted as aliases.

Each supported source slice is selected explicitly; wrong, absent or abbreviated revisions remain Unknown. Unlisted metrics are not assigned semantics. A missing or malformed required family remains Unknown rather than zero.

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
- Fractional cumulative transfer seconds are valid; fractional byte/token/key/event
  counts are not. Residency gauges retain byte units rather than being treated as
  stored-token counts. Neither reported residency nor configuration proves actual
  physical allocation, capacity or tier participation.
- Initial duration-histogram slices cover vLLM offload synchronous lookup and
  LMCache retrieve operation time. `_bucket`, `_count`, and `_sum` use the exact
  parent histogram TYPE and UNIT, never scalar-suffix inference. Buckets/counts
  count observations; sums carry seconds, including fractional seconds. Bucket
  `le` is required and must be a finite nonnegative bound or `+Inf`. Late or
  contradictory parent TYPE is Unknown. These are reported operation times, not
  client TTFT/prefill latency or automatically transfer time. Partial families
  never establish a complete distribution; no quantiles are synthesized.
- Gauge changes are not counter deltas. Reported window hit rates are not
  same-cohort phase hit rates. A known counter zero is different from a missing
  counter, and neither alone proves physical-cache absence or participation.

## TRT-LLM collector caveats

The source-pinned `trt-llm` slice is not a Triton backend metric alias map. Its
manager block counters accumulate only positive deltas from ordered per-rank
snapshots: a decrease changes the exporter baseline without decrementing the
counter. Monotonic exposition is therefore not independent cache-manager
continuity or reset evidence. Manager and iteration views overlap; do not sum.

Connector reuse blocks and connector-attributed prompt tokens count accepted
loads **before asynchronous transfer completion**. Tier attribution preserves
private `dp_rank`, `cache_level` and `cache_tier` identity: two levels can share
a device-memory tier name, and already-prefetched disk pages count as host.
These are reported attribution/acceptance, not proven completed transfers or
physical-tier participation. The public projection does not expose arbitrary
tier/model label strings. No query denominator is invented from miss counts.

Host-to-device onboard bytes, device-to-host offload bytes, intra-device copy
bytes and disk-prefetch blocks retain distinct metrics/units. Max/free/used-block
gauges and tokens-per-block layout do not establish physical occupancy or permit
block-to-byte conversion without dtype/layout/parallelism evidence. Cached-token
counters have process lifetime, not cache-flush lifetime. Missing optional
families stay unavailable; a missing required cached-token family invalidates
the selected profile evidence rather than creating zero.

## Machine-readable capability boundaries

Each capture includes `cache-metric-capabilities.v1`: expected metric names,
family types, units/roles, required-family and cumulative flags, and histogram
parent identity/unit separate from bucket/count observation units. These are
source-reference descriptors, not assertions that a server exported them.
Operator-declared source compatibility is separately `not_declared`,
`matches_reference` or `unsupported`; a match does not verify the executable.
Release aliases remain empty, legacy Dynamo's restriction remains explicit,
and the TRT-LLM and `4ba787ec` vLLM slices record explicit semantic qualifiers. Complete-profile support, physical-tier validation,
phase-delta validation, complete-distribution validation and quantile
reconstruction are explicitly false in this prototype.

## Opt-in startup discovery (development)

Both runners accept `--cache-metrics-url`, `--cache-metrics-profile` and optional
`--cache-metrics-source-revision`. Endpoint and profile must be selected together;
a supplied revision must be a full source SHA. Missing/unsupported revisions
preserve Unknown sample values rather than guessing executable compatibility.
Profile choices are `vllm-apc`, `vllm-external`, `vllm-offload`,
`vllm4ba-apc`, `vllm4ba-external`, `vllm4ba-offload`, `sglang`, `sglang0523`, `lmcache`, `lmcache057-mp`,
legacy `dynamo-kvbm` and `trt-llm`. These are selections, not backend autodetection.
Stable standalone 1.2.1 uses the separate `trt-llm121` selection; it does not
export the newer collector's cached-token or KV reuse families.

Discovery performs one unauthenticated GET before measurement. It sends no
workload body or inherited inference API key, refuses URL credentials, disables
redirects, and bounds the response to 4 MiB with a five-second fetch deadline.
HTTP, size, UTF-8, deadline and parser failures are coded unavailable evidence;
they do not become zero counters. No flags means no discovery GET or new metadata.
Protected exporters need a separately designed explicit authentication path;
inference credentials are deliberately not forwarded.

The redacted capture appears under `native_cache_metric_discovery` in strategic
`config` or LLM effective modality configuration. It retains expected family
type/units, missing-family lists, per-sample unknown reasons, source declarations
and fetch timestamps. It excludes endpoint URLs, raw invalid values and private
worker/model labels. Only validated bucket bounds and whitelisted SGLang cache
sources are projected. Opaque series IDs are scoped to one snapshot; full private
labels remain in a non-serialized probe for later same-source phase matching.
There is no replica/tier summation, distribution reconstruction or request/tier
attribution. Discovery values are cumulative endpoint observations, not phase
counter deltas, rates, hit ratios or client timing measurements.

Remaining work includes captured-exporter fixtures, validated release aliases
and broader capability coverage, broader histogram coverage and complete-family distribution validation,
residency/capacity/eviction/failure coverage and broader TRT-LLM/Triton scope. Full telemetry-phase
integration and phase-scoped deltas/reset/identity handling belong to subsequent
work; none is inferred from a startup snapshot.
