<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Explicit cache workload policy and effective order

The opt-in `cache-workload.v1` extension is shared by LLM chat and strategic chat.
It describes payload mutation/order, **not** a server hit, cache reset, eviction
or offload validation. No live engine or performance claim follows from offline
payload tests.

## Policy

| CLI policy | Serialized policy | Behavior |
| --- | --- | --- |
| omitted / `legacy-unspecified` | `legacy_unspecified` | Preserve historical controls/defaults; not an assertion of reuse or isolation |
| `reuse` | `reuse` | No cache-policy mutation after existing body construction |
| `bust-request` | `bust_request` | Distinct nonce for each request in an invocation |
| `isolate-run` | `isolate_run` | Stable nonce within an invocation, different across invocations |

Explicit cache policy requires chat and rejects conflicting historical mutation
flags: LLM `--unique-prompts`, or strategic `--prefix-control unique`. Reuse
preserves the fully built payload, including deliberate hints/system messages;
it does not remove them or guarantee eligibility/hits. Legacy controls still work
unchanged when no explicit policy is selected.

New nonces prepend the **first message content**, keeping the rest of the history
intact. The prefix is `[kv-<16 lowercase hex characters>] `: 64 bits from the
same identity SHA-256, adding exactly **22 ASCII characters**, not a full 64-hex
digest. This is probabilistic separation, not a collision-free namespace.
Common chat-template tokens before that content can still be cached; templates
with long built-in system headers can share **at least one full block** even
under bust-request or isolate-run. Neither policy implies zero shared blocks.
When prefix validation runs, retain the native pool maximum for Pass **or Fail**;
Unknown/unrequested validation has a null residual, never fabricated zero.

The body-free `cache-policy-effects.v1` extension records character overhead,
nullable signed token overhead, token scope and nullable residual-block evidence.
Pinned full rendering measures after-minus-before token counts in the same
identity and binds the after-render to the exact prepared body digest. It is not
the token count of a standalone nonce: merges at content boundaries can change
the contextual difference. Without that rendering the token delta is unknown.
Keep overhead in input-length distributions; do not compare mutated request
lengths as though they were the unmodified baseline. The #421 manifest accepts
per-body `workload.policy_effects` entries (duplicates retained for issuance),
with explicit unknowns; older manifests remain valid. Full rendered token
differences require recorded tokenizer/template identity in the manifest. The
per-body before/after rendered token counts retain the actual input-size change.

With held-out pools and an explicit policy/order, all stage bodies are frozen
before traffic/hooks, including the nonce, then rendered/validated and sent
without another mutation. `nonce_identity_sequence` is the planned nonce ordinal
for bust-request, not necessarily the request-row sequence on a stopped run;
ordinals remain distinct across warmup, measurement and stages. Isolate-run uses
one invocation nonce across all stages. `config.warmup_stages` retains every
stage's final-body proof rather than reusing the first stage's nonce digests.
Planning bounds are 100,000 bodies, 64 MiB of planned wire bytes and 10,000,000
baseline/final rendered tokens across these policy stages; exceeded bounds fail
before hooks/HTTP. Unconfigured legacy paths retain their historical behavior.

This is not a claim that all root reuse is eliminated. No nonce clears existing
tier occupancy. Mutation currently requires string first-message content, and
unsupported content fails preflight rather than being converted lossily.

## Effective order and replay

Both runners accept `--prompt-order preserve|seeded-shuffle|prescribed` and
`--order-seed` (defaults to the historical seed). Legacy LLM defaults to seeded
shuffle; strategic defaults to preserve unless `--shuffle-prompts` is supplied.
Thus equal seeds alone still do not establish equivalence. Explicit policies use
the same ordering implementation. Model, body controls, temperature, streaming,
system prefix, prompts and output-token limits must also match for equal wire bodies.

`--prompt-sequence FILE` reuses the existing fingerprint-first prescribed sequence
format. Content mismatch fails before traffic; a matching prescription takes
precedence over shuffle seeds, but conflicts with explicit preserve/shuffle order
are rejected. Index-only rows remain range-validated. Sequence logging resolves prescribed indices against the original pool, not the
expanded/reordered issuance pool. This preserves original indices when a small
pool is repeated and safely handles a short prescription with a high source
index; source indices must not be applied a second time to the issuance pool.

Strategic `--prompt-sequence` / `--prompt-sequence-log` currently require
`--prompts`. The log describes the canonical measured sequence **per stage**;
measured indexing restarts at zero each stage and warmup has separate accounting.
Session turns cannot be flattened by a prompt prescription or shuffled out of
order. Explicit payload policy also applies to closed-loop recorded-history
issuance: request-scoped nonces span warmup and measurement while original turn
positions/history and the post-drain reset boundary remain intact. Canonical
ticketing covers both legacy and closed-loop scheduling paths. This does not promise deterministic physical insertion/eviction order:
concurrency, routing and feedback-driven session completion remain factors.

LLM `--arrival-seed` is independent of order (historical default: seed + 1).
Strategic rate issuance is deterministic and does not advertise a nonexistent
arrival RNG. Input selection/transformation provenance remains a separate control.

## Exact-body evidence

Under explicit cache/order controls, serialize once **after** all body transforms
and send those same bytes. SHA-256 covers HTTP JSON body bytes, excluding headers
and transport framing. Retain hashes, not prompts. Request failures retain intended
body identity, not proof that a server received or cached it.

- LLM request `modality_labels` contains `cache_workload_version=cache-workload.v1`
  and `body_sha256`. This is a versioned extension of the existing arbitrary label
  map; core request schema versions do not change.
- Strategic CSV appends optional `cache_workload_version` and `body_sha256`
  columns. Historical columns keep their positions; old CSV inputs default the
  new fields to absent. These fields are not yet carried by telemetry RequestRow.
- LLM effective common arguments retain `modality.cache_workload`; strategic
  retains `config.cache_workload`. Metadata records effective policy/order/seeds,
  nonce scope/placement, digest counts and client serialization timing.
- Body-fingerprint-list SHA-256 hashes a compact UTF-8 JSON array of body digest
  strings in canonical issuance sequence, preserving duplicates and errors.
  Effective-order SHA-256 hashes an ordered array of fallback request IDs
  `body:<sha256>`; future canonical inputs must retain their dataset IDs separately.
  Strategic per-stage evidence includes warmup; final evidence covers the run.
  Do not confuse desired issuance with observed server scheduling.

To preserve the legacy measurement path, omitted controls still serialize inside
the send/service clock. Explicit cache/order controls prepare the bytes before
that clock; metadata discloses this client-side serialization difference. It is not isolated
server prefill time, and tiny latency differences between these paths must not be
marketed as cache performance.

Body digests do not establish token-prefix disjointness. A pinned tokenizer/chat
renderer must bind its rendered tokens to these exact bytes; any policy/body
change invalidates earlier token validation. Digests are not privacy clearance,
and existing debug logging can include payload content: do not publish those logs.

## Offline validation boundary

Golden unit tests cover reuse, request/run mutation scope and deterministic order.
A small local byte-tokenizer/chat-template fixture quantifies remaining common
framing and nonce-prefix tokens; it is not a production-model isolation certificate.
The mock integration suite compares received unary/SSE bytes and recorded digests across
both runners, including preserve/shuffle/replay, distinct shuffle seeds under
replay, short-prescription logging, mismatched replay rejection and invocation
isolation. A session/reset regression additionally checks original turn positions,
recorded rather than generated assistant history, per-request nonce uniqueness
across warmup/measurement, and exact received-byte digests. These prove client
payload contracts, not physical cache participation.
