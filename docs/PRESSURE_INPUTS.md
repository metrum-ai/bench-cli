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

This is a foundation for broader pressure families/source trace adapters, not full
trace replay, cyclic/Zipf generation or physical multi-tier validation.
