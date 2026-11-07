<!-- Copyright (c) 2026 Metrum AI, Inc. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Runtime renderer consumption (development unit)

This is a consumer of the producer-owned [offline renderer](CHAT_RENDERING.md),
not another template/tokenizer implementation. It requires the `tokenizer`
feature. Strategic static held-out pools opt in with
`--warmup-render-config PATH`. The local JSON file supplies tokenizer/template
paths (relative to that file), `identity`, `special_tokens`, and explicit
`max_artifact_bytes`, `max_body_bytes`, `max_rendered_bytes`, `max_tokens`, and
`template_fuel` limits. Identity uses the producer's `RenderingIdentity` shape.
The config is capped at 64 KiB; no remote artifact fetch is performed.

This freezes exact serialized warmup/measured request bytes at setup and reuses
the existing prebuilt-byte HTTP issuance path. Both pools are rendered and passed
to the authoritative overlap validator before any stage traffic or reset hook.
Fail/Unknown abort even if `--allow-unverified-warmup` was supplied. Without this
opt-in, legacy preparation and the explicitly non-certifying Unknown override
are unchanged. Closed-loop held-out mode remains guarded until dynamic final-body
bindings are implemented. Rendering/serialization is outside stage timing and
this timing change is recorded in warmup metadata. Proof budgets cap the planned
issued bodies at 100,000 and aggregate rendered tokens at 10,000,000. Repeated
static pool entries reuse their validated tokenization; request IDs, body-digest
multiplicity and aggregate token accounting still cover every planned issuance.
`body_binding_scope` discloses planned static per-stage coverage: stopped/error
runs can issue only a subset. The proof is not a statement that every planned
request reached the server.

Warmup config metadata records the producer overlap population/truncation fields
and fingerprint-only per-body rendering evidence. Live engine tokenization is
not yet wired to the CLI: status is explicitly NotRequested, not a fabricated
match. Final study-report integration is separate.

`runtime_rendering::prepare_wire_body` accepts owned final HTTP bytes after every
history replacement, hint, nonce and cache transformation. It computes SHA-256
independently, calls `chat_rendering::render_wire_body` with pinned local artifacts
and explicit limits/context, checks the producer digest and identity, and builds
the producer's `token_overlap::RenderedBody`.

`PreparedWireBody` retains these exact immutable bytes. Issuance must use its
`wire_bytes()` with the existing authentication/headers and explicit JSON content
type, not reserialize a JSON value or mutate history later. A later mutation
requires new preparation, rendering and validation. Wire bytes/token IDs are
neither Debug nor Serialize on the prepared container.

## Optional live cross-check boundary

A selected engine adapter must compare token IDs for the **same exact body and
formatting policy**, not token counts or prompt text without framing/tools. When
requested, status starts Pending and overlap tokens are unavailable. Match makes
them eligible; Mismatch or Unavailable latches for that prepared body and clears
the tokens projected to the authoritative validator. The validator therefore
returns Unknown, never an isolation Pass or a fabricated cache miss. Later matches
cannot erase a contradictory result. An HTTP engine adapter is not implemented by
this unit. It must bound responses, reject invalid/noninteger IDs and record
failure rather than silently skipping a requested check.

`kv-wire-rendering.v1` evidence contains wire/rendered-text hashes, pinned
rendering identity and cross-check status. It excludes raw bodies, arguments,
special-token bindings and token IDs. The caller must use this finalized state
when constructing report/overlap evidence. A matching tokenization does not prove
physical cache residency, cache hits, backend version authenticity or performance.
The artifact revision is an operator declaration, not independently authenticated
by its file checksum.

The synthetic local BPE fixture checks exact-byte retention (including JSON
whitespace), distinct wire/rendered hash bases, metadata privacy, logical overlap,
Pending/Match/Mismatch/Unavailable behavior, failure latching and artifact mutation
rejection. It does not establish any engine's real tokenization compatibility or
physical cache state. Tool-aware rendering remains producer-owned; unsupported
templates fail without a fallback algorithm.
