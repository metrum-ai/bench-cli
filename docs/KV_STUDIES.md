<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# KV study manifest/report contract

`kv-study-report.v1`, specified by [the JSON Schema](../schemas/kv-study-report.v1.json), is an
**opt-in interchange contract**, not an additional field in existing runner
summaries. This first contract does not implement session lifecycle, metric
profiles, counter deltas, cohort tables or CSV/HTML generation. Those extensions
must consume the same phase identities and provenance; new fields require a
new schema version. Existing request and summary schema versions are unchanged.

## Evidence and unknowns

A manifest records model/tokenizer revisions, rendered chat-template digest,
engine/connector versions, runtime flags digest, block size in tokens, KV
layout/dtype, parallelism, routing and tier capacities in bytes. `null` means
unknown, not zero or unsupported. Tier capacity evidence distinguishes observed,
declared and unknown. Do not include endpoint addresses, command secrets, raw
prompts, user identifiers or credentials. Digests are integrity evidence, not
privacy clearance.

`reuse_kind` distinguishes exact-prefix reuse from approximate chunk reuse and
compression. Approximate/compressed studies need separate quality, recompute and
codec measurements; they cannot be advertised as exact prefix hits. All studies
are engineering measurements, not certification under an inference standard.

## Dataset interface (#424 / #428)

`datasets[]` is a small source envelope: source ID, resolved immutable revision,
config/split, selected files (`path`, `sha256`, `bytes`), `adapter_id`,
`adapter_version`, and `license_spdx` (null if unknown).
`validation_manifest` references the **separately versioned** ingestion validation
artifact (`kv-dataset-manifest.v1`) by its `schema_version` and `sha256`. This deliberately does not invent a
second rights/token validation schema. That artifact owns field mappings,
canonical `record_id` / `session_id` / `trace_id` / `turn_index` identity,
component/row licenses, terms and gate
policy, privacy exclusions, conversion versions, corpus/sample counts, tokenizer
and template pins, rendered overlap and selection diagnostics. An unavailable
artifact is `null`, never an implied rights approval. Synthetic inputs may use
an explicit synthetic source ID and null revision. Publication requires complete
pins and reviewed provenance, not merely schema validity.

## Effective workload

Record cache mutation policy, effective order and fingerprints **after** all
transformations. `reuse` means no hidden mutation, not a guaranteed physical
hit. User-text busting may leave the system/chat root reusable. Legacy defaults
remain `legacy_unspecified` unless explicitly established.

The effective-order digest is SHA-256 of the UTF-8 JSON serialization of an
ordered array of canonical request IDs, without whitespace. Body fingerprints
are SHA-256 of the UTF-8 JSON serialization of an ordered array of SHA-256 body
digests. Each body digest hashes the exact serialized JSON request bytes sent
after transformation (not headers); transport-only differences are excluded.
IDs must be non-sensitive stable opaque IDs. Repeated IDs remain repeated.
These digests are not interchangeable with token-prefix overlap measurements.

Record recorded-history versus live-generated history and open-loop versus
closed-loop arrival mode. Recorded assistant answers may differ from generated
answers and do not prove reuse of generated KV. Distribution descriptions must
name units, actual input/output counts and controls, not just requested targets.

## Phase and reset contract (#413)

Each phase has a unique ID within a study and is warmup, cold/populate, resident
warm, pressure or re-hit. Phase kind states intent; `cache_state` states evidence.
A cold/populate phase with unknown state is valid but **not a cold result**.
A verified-cold claim requires a verified reset with nonempty tier scope,
acknowledgement, readiness and transfer settling. Schema validity alone cannot
verify that every intended tier was actually reset or that evidence is truthful.

Warm kernels/connections with token-prefix-disjoint held-out inputs, then reset
intended tiers for cold measurement. Different files or session IDs do not prove
prefix isolation. Drain requests and asynchronous transfers before boundaries.
Reset hooks are opt-in trusted operator commands, never dataset commands. Fail
closed on timeout, nonzero exit, JSON `success=false`, readiness failure or
unacknowledged tier reset; HTTP success alone is insufficient. Failed/unverified
resets must not be relabelled cold. This contract does not execute hooks.

## Accounting and downstream exports

Phase `requests` counts all issued requests, including errors. `errors` is a
subset; `cached_token_known` and `cached_token_unknown` partition **all** phase
requests, including errors. Missing, null, malformed or contradictory cached-token
usage is unknown, not an observed zero. Consumers must enforce
`known + unknown == requests`, `errors <= requests`, and unique phase IDs;
JSON Schema cannot enforce these cross-field arithmetic constraints.

Future tables must separate successful measured requests from warmup and errors,
preserve unknown coverage, and distinguish client overlap ceilings from reported
cached tokens, aggregate counters and directly observed tier evidence. Report
first-any and visible TTFT separately; neither is isolated server prefill time.
Never assign aggregate external hits to individual requests. Phase counter rates
require matching per-series identities, valid denominators and complete boundary
coverage; lifetime counters are not measured-phase ratios.

Use paired independent reset repetitions with randomized/interleaved arm order.
Reuse repeat/bootstrap infrastructure with run/session cluster resampling, not
independent-turn resampling. Retain n/errors, uncertainty and small-n tail warnings.
Hold model, tokenization/template, output lengths, routing, chunked-prefill and
batching fixed or disclose factors. Open-loop results retain scheduled arrivals,
backlog and concurrency-cap engagement. JSON/CSV/HTML must eventually reconcile
counts, units and provenance; the present milestone only defines JSON interchange.

## Validation boundary

Offline tests validate version/required fields, digest shape, reset claim gates
and an unknown-usage mock response. They do not validate a real engine, reset,
cache transition or performance. Full integration must separately exercise cold
miss → resident hit → eviction → external hit → miss/recompute, failed resets,
unknown telemetry and actual sequential turn issuance. No live support or speedup
claim follows from these fixtures.
