<!-- Copyright (c) 2026 Metrum AI, Inc. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Canonical runtime JSONL inputs

`canonical_input::load_canonical_jsonl(path, IngestionLimits)` returns
`Result<Vec<canonical_input::CanonicalRecord>>` from a **local** bounded file.
This is the runtime interface for session/order consumers. Legacy loaders that
return `Vec<String>` are unchanged. Rich `public_dataset::IngestionRecord` values
remain source-stage records; the checked conversion and its required envelope
context are documented in [PUBLIC_DATASETS.md](PUBLIC_DATASETS.md). Never
serialize source metadata into user text.

## Exact runtime fields

```rust
pub struct CanonicalRecord {
    pub record_id: String,
    pub source_row_id: String,
    pub session_id: Option<String>,
    pub turn_index: Option<u32>,
    pub group_id: Option<String>,
    pub trace_id: Option<String>,
    pub arrival_offset_ms: Option<u64>,
    pub body: CanonicalBody,
    pub expected_output_tokens: Option<u32>,
    pub source: DatasetEnvelopeRef,
}

pub enum CanonicalBody {
    Prompt(String),
    Chat(Vec<Message>),
}
```

Prompt-only inputs remain `CanonicalBody::Prompt`; chat inputs remain
`CanonicalBody::Chat`. Serde uses externally tagged bodies: `{"Prompt":"text"}`
or `{"Chat":[{"role":"user","content":"text"}]}`. There is no top-level
`messages` or `output_budget` field. `source_row_id` is required, nonempty source
lineage; multiple derived targets may refer to the same source row. Preserve it
separately from `record_id`, without inserting metadata into request text.
`turn_index` is a zero-based u32 source turn index; overflow fails at load.
`expected_output_tokens` is a positive u32 target, not observed output.
`arrival_offset_ms` is relative u64 milliseconds (None/null when unknown), not an
inferred time unit. Both it and `source: DatasetEnvelopeRef` are required struct
fields; `source` cannot be null or omitted in JSON. Every target preserves
record/source-row/session/group/trace IDs; scheduling must carry them unchanged.
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

For `CanonicalBody::Chat`, messages use roles `system`, `user`, `assistant`, `tool`; `content` is a string or
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
