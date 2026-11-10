<!-- Copyright (c) 2026 Metrum AI, Inc. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->
# Logical capacity-aware pressure plans

`pressure_inputs` is a source planning/modeling library, not a runtime scheduler
or a cache measurement. It builds populate-A → distinct-pressure-B → re-hit-A
plans. Configured capacity must be known to label below/above capacity. Below is
strictly below; equality is neither band. Above requires known complete cacheable
blocks, not uncertain partial tails. Hot population must fit configured capacity.

Classic K/V geometry computes per-rank block bytes as
`2 * effective_layers * kv_heads_per_rank * head_dimension * dtype_bytes * block_tokens`,
then uses the bottleneck rank's usable cache capacity. Effective heads are explicit;
there is no blind division by tensor parallelism when KV heads replicate. All
products are checked. Missing geometry remains unknown; non-classic layouts must
supply an explicitly established capacity rather than misuse this formula.

Plan node IDs denote globally interned prefix-tree nodes, NOT unordered individual
hash labels. The same label under different parents must get distinct node IDs.
The LRU oracle rejects conflicting node parents, has explicit capacity/distinct-node
limits and deterministic eviction. Its hits count individually resident logical
nodes, not a backend's contiguous-prefix lookup policy or saved prefill tokens.
Unknown capacity yields unknown simulation,
not a successful hit/residency assertion. The oracle is CPU-only modeled behavior.

Every plan keeps `rendered_token_overlap=unknown` and physical cache evidence
unavailable. It contains no reconstructed original text, actual token IDs, observed
resident/tier hits, measured eviction, saved prefill or offload claims. Model output
must never be renamed observed telemetry. Runtime owns actual scheduling, achieved
order/timing, body rendering and warmup isolation. Length/time-only source inputs
cannot establish overlap. A later constructed replay must independently validate
actual rendered tokens against its pinned tokenizer/template.

`modeled_tiers::simulate` is an additional exclusive two-tier LRU oracle: primary
eviction demotes, external hit promotes, and secondary eviction drops a node.
Tests distinguish resident hit, external hit and miss/recompute; unknown capacity
remains unknown. These are CPU model outcomes, never observed tier transfers or
physical cache behavior. A backend's admission/prefix/tier policy may differ.

## Constructed cyclic/integer-Zipf families

`pressure_families::generate` constructs independent full-node group chains and
reproducible logical access order with explicit seed, node/request/group limits,
requested pool, achieved group counts and achieved distinct footprint. It does
not synthesize text or final token IDs, invoke a source script, issue traffic,
set arrival times or guarantee real pressure/eviction. Rendered overlap remains
Unknown and physical evidence unavailable.

- Cyclic order rotates through all configured groups; the seed selects initial
  rotation. First accesses populate; revisits are re-hit candidates.
- Integer Zipf has exponent exactly one and weights `floor(weight_scale/rank)`;
  scale must preserve positive support for every group. This is not an arbitrary
  real-valued exponent or a production distribution fit. SHA-derived bounded
  rejection sampling avoids modulo bias and floating arithmetic; exact effective
  order/counts are reported, not assumed to equal requested weights.
- Requested pool above configured capacity does NOT mean the achieved footprint
  exceeds capacity. Both relations are reported independently, with explicit
  below/equal/above/unknown values. Missing capacity remains Unknown.
- Feed the logical requests to the CPU-only LRU oracle for **model** behavior.
  Distinct-node totals are not instantaneous residence or contiguous-prefix hits.

Artifact schema: [logical-pressure-family.v1](../schemas/logical-pressure-family.v1.json).
Numeric count/order/path consistency is checked on construction, beyond JSON
Schema shape. Deserialization alone is not validation or access authorization.
The plan fingerprint covers semantic version, config, effective order/counts and
actual logical request chains. It is not an issued-HTTP-body digest.
Canonical request generation and final rendered-token checks remain separate.

These are foundations, not full trace replay or physical multi-tier validation.
