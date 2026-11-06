<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Canonical workload consumption boundary

`--canonical-jsonl` selects a separate, bounded canonical input path. It never
falls back to legacy prompt parsing. Legacy `Vec<String>` loader signatures and
defaults remain unchanged. The authoritative `canonical_input` loader checks the
bounded local file before HTTP traffic, including pre-warmup reset hooks. Invalid
canonical input never becomes a legacy prompt file. Structural validation and
provenance references do not establish legal clearance or independently verify
the referenced source artifact.

## Final record interface

The loader-owned `CanonicalRecord` retains `record_id`, `source_row_id`, optional
`session_id`, `turn_index: Option<u32>`, `group_id`, `trace_id`,
`arrival_offset_ms`, `body: CanonicalBody`, `expected_output_tokens` and
`source: DatasetEnvelopeRef`. `CanonicalBody` is `Prompt(String)` or
`Chat(Vec<Message>)`. Turn overflow is rejected, never truncated. Typed messages
preserve inert tool-call metadata and results without executing a tool.

Consumption-owned `WorkloadInput` distinguishes `Legacy(Vec<String>)` from
`Canonical(Vec<CanonicalRecord>)`. The consumer does not fetch sources, infer
rights, substitute provenance or implement another ingestion algorithm.

## Prepared targets and controls

A canonical record is already a selected request target. A chat record becomes
one complete wire history, not every prefix again. Prompt-only records become
one user message. Optional role selection checks the target's terminal role;
canonical identities and turn indices are not renumbered. Structured tool
calls, arguments and result IDs are retained as data; missing function payloads
must never be reconstructed from opaque call IDs.

Output controls remain explicit: `--max-tokens` is required. Source
`expected_output_tokens` is retained as metadata and is not silently substituted
for an operator's output-token limit. Shared-prefix, temperature and other
explicit body controls still apply; final-body/token evidence must bind their
post-transformation output rather than the source body alone.

Closed-loop scheduling honors `session_id` globally within the input, matching
the loader's `(session_id, turn_index)` uniqueness rule. A change of source
envelope must not create a second overlapping cursor for the same session.
Original source IDs remain unchanged in exports. Every closed-loop target must
have a session ID. Source turn numbering is preserved separately from
legacy `turn`, which remains the original one-based message position.

Source arrival offsets are retained unchanged as metadata. The current stage
scheduler is not a source-trace replay scheduler: a warning and
`canonical_input.arrival_offsets=metadata_only_not_trace_replay` disclose this.
Likewise, canonical source order does not accept the flat legacy
`--shuffle-prompts` switch. These declarations must not be described as achieved
trace timing or cache insertion order.

## Lineage extension (`canonical-lineage.v1`)

The consumer carries every identity, source-row reference, desired arrival,
expected output target and the exact dataset envelope without messages/prompts.
Strategic CSV appends `canonical_lineage_json`, a JSON-encoded lineage cell; old
CSV inputs default it to absent. Config retains deduplicated `datasets[]` exact
envelopes and the canonical ordering/history/arrival-control declarations.

No raw message body is included in lineage. IDs and source metadata still require
normal privacy review; a digest or identifier is not publication clearance.

## Validation boundary

Offline tests cover overflow rejection, body-free lineage, target preparation,
typed tool payload preservation, global session identity across source envelopes, and CSV
identity round trips. The invalid-schema mock asserts both zero HTTP traffic of any method
and no reset-hook execution, even if the file is valid in the legacy format.
A real-file CLI mock checks one retained target rather than repeated history
expansion, exact wire messages, unchanged lineage and explicit output controls.
These tests do not establish source rights, live engine support, physical cache
participation, or token-prefix isolation. Held-out closed-loop warmup still fails
closed until dynamic final-body bindings are integrated; an override cannot
certify unbound data.
