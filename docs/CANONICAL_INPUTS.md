<!-- Copyright (c) 2026 Metrum AI, Inc. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Canonical runtime JSONL inputs

`canonical_input::load_canonical_jsonl(path, IngestionLimits)` returns
`Result<Vec<canonical_input::CanonicalRecord>>` from a **local** bounded file.
This is the runtime interface for session/order consumers. Legacy loaders that
return `Vec<String>` are unchanged. Rich source-adapter records in `public_dataset`
remain ingestion-stage records; convert them explicitly to this runtime contract
rather than serializing source metadata into user text.

## Exact runtime fields

- `record_id: String`
- `session_id: Option<String>`
- `turn_index: Option<u32>` (zero-based source turn index)
- `group_id: Option<String>`
- `trace_id: Option<String>`
- `arrival_offset_ms: Option<u64>` (relative milliseconds, not an inferred time unit)
- `messages: Vec<canonical_input::Message>`
- `expected_output_tokens: Option<u32>` (positive target, not observed output)
- `source: DatasetEnvelopeRef`

Represent a single prompt as one `user` message. Every target preserves opaque
non-sensitive record/session/group/trace IDs; scheduling must carry them unchanged.
The loader preserves source order and rejects duplicate record IDs or duplicate
(session_id, turn_index) pairs. A turn index requires a session ID. It does not
schedule, shuffle, infer arrival times or execute consecutive turns. Runtime owns
sequential execution, actual arrival order and live/recorded-history policy.

`DatasetEnvelopeRef` uses exactly the report's `datasets[]` envelope fields:
`source_id`, `resolved_revision`, `config`, `split`, `files[{path,sha256,bytes}]`,
`adapter_id`, `adapter_version`, `license_spdx`,
`validation_manifest{schema_version,sha256}`. Unknown optional metadata is null,
not an empty string or a fabricated revision. `validation_manifest` may itself
be null. References use `schema_version: "kv-dataset-manifest.v1"`.
Converting a complete ingestion `DatasetEnvelope` wraps its known fields without
renaming or dropping any field.

The loader validates envelope structure and digest shape, but does **not** fetch
or verify the referenced artifact, infer rights/privacy approval or certify a
source's revision. Unknown rights remain unknown; public use must separately pass
the ingestion/provenance policy. Structural validity is not rights clearance.

## Typed inert history

Messages use roles `system`, `user`, `assistant`, `tool`; `content` is a string or
null. Multimodal message arrays are unsupported by this text-only contract.
Optional `tool_calls` contain `id`, `type: "function"` and
`function{name,arguments}`; arguments remain a JSON object encoded as a string.
Tool results use `tool_call_id`. Names/arguments are never dispatched or evaluated.

Tools must be called by an assistant after user history, uniquely identified,
matched exactly once, and resolved before a non-tool message. Tool calls cannot
remain pending at the request boundary. System messages precede conversation;
assistant-only/tool-only prefixes fail. Requests end at a user or resolved tool
result, **not at the current/future assistant reference answer**. This structural
check cannot detect every semantic answer leak embedded in source text.

Recorded assistant history may be present before the final request point, but
must not be silently presented as live generated history. Source adapters keep
future reference responses separate and runtime inserts actual generated history
where required; this loader never synthesizes assistant placeholders.

## Bounds and diagnostics

Per-file/aggregate byte limits and row limits apply before/while JSONL decoding.
Blank/malformed rows, empty workloads/messages, unknown roles/fields, out-of-range
u32 turn/output values, invalid tool pairing and bad digest references fail before
traffic. Error messages identify the row without echoing raw messages, unknown
role values or tool arguments. There is no HTTP fetch, Hub fallback or remote
code execution. Local unit fixtures require no credentials or network access.
