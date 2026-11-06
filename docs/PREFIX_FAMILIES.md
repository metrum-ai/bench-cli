<!-- Copyright (c) 2026 Metrum AI, Inc. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Deterministic prefix families

`prefix_families::generate` constructs a bounded token-first synthetic profile and
uses [the shared rendered-token validator](PREFIX_VALIDATION.md). It is a library
API, not yet a CLI flag or scheduler integration. No source corpus is required.
The generator version is `prefix-families.v1`; its configuration, rendering
identity, effective rows, group counts and measured output are fingerprinted.

## Configuration and units

`prefix_tokens` and `suffix_tokens` are **final rendered tokenizer IDs**, not
characters or supplied source targets. The prefix includes an explicit common
chat root. The unique ordered `token_pool` excludes any unwanted special IDs;
the generator does not infer a safe vocabulary from an arbitrary model.
`output_budget` is a target, not a measurement of generated output.
`max_total_tokens` bounds construction before allocation/rendering.

`reuse_fraction_ppm` selects floor(request_count * fraction / 1,000,000) requests
for shared families; remaining requests have singleton groups as a no-reuse
control. Seed each shared family once, then allocate remaining family requests
uniformly or with fixed-point Zipf-1 weights floor(1,000,000 / rank).
`requested_family_requests` is not a hit rate. `achieved_repeated_requests` counts
requests beyond the first in each group; these differ because first uses populate
a family. Finite sample group counts are always recorded.

Group branch IDs are chosen without replacement so different groups share only
the explicitly declared common root. Within each group, suffix first IDs are
chosen without replacement so the exact LCP is the requested prefix length.
Insufficient branch/suffix vocabulary, too few requests for groups or a token
budget overflow fails deterministically, rather than silently reducing diversity.
A zero-token suffix deliberately creates full-body repeats; duplicate counts stay
visible. A zero-token prefix is supported for a single root-free group.

The same seed, configuration and rendering artifacts reproduce identical records
and order. Optional seeded shuffling changes order, not group identity. Generator
fingerprints hash compact JSON with the `fingerprint` field initially empty;
consumers must use that same convention when checking the artifact.

## Rendering contract

The caller supplies a local pinned-tokenizer/template renderer that decodes the
planned tokens, fully renders the wire input, and re-encodes it. Any resulting
ID drift fails. Do not use a token identity callback as evidence that a real
chat template/tokenizer works; that callback is only for offline token fixtures.
Common framing tokens must be represented in `common_root`. Generation does not
execute remote code or invoke an inference endpoint. Arbitrary BPE vocabularies
may fail decode/re-encode identity; such failures are intentional safety gates,
not an excuse to advertise requested lengths as measured lengths.

The final validator records measured lengths, within/across-group LCP and
block-aligned ceilings. It proves neither server-window feasibility nor cache
residency, physical tier hits, eviction or offload performance. Future pressure
scheduling must preserve the generated group IDs and effective fingerprints.

Unit fixtures cover zero/full/mixed family fractions, uniform/Zipf allocation,
multiple groups, zero prefix, whole-body repetition, deterministic ordering,
impossible configurations and explicit render/re-encode drift rejection.
