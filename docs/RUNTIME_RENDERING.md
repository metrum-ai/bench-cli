<!-- Copyright (c) 2026 Metrum AI, Inc. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Runtime renderer consumption (development unit)

This is a consumer of the producer-owned [offline renderer](CHAT_RENDERING.md),
not another template/tokenizer implementation. It requires the `tokenizer`
feature. It is not yet wired to runner CLI, HTTP issuance or study reports;
existing defaults and the closed-loop held-out guard remain unchanged.

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
