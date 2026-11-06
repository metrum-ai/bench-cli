<!-- Copyright (c) 2026 Metrum AI, Inc. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Public dataset contract (v1)

The `public_dataset` library module defines the initial shared ingestion and
provenance contract. It is not yet a CLI adapter catalog or runner integration.
Existing prompt-library behavior remains unchanged.

## Pinned transport and cache

`DatasetOrigin` names an exact dataset ID, 40-hex commit revision, config, split,
and ordered source files with expected byte counts and SHA-256 digests. Floating
refs must first be resolved using the existing prompt-library revision resolver.
`materialize` reuses its authenticated Hub request headers. Only explicit Hub
files are requested; scripts, repository code and ingested tools never execute.
No mirror substitution or automatic agreement acceptance occurs.

Each file is streamed through a byte limit and SHA-256 verification into a
unique temporary file, then installed under its content digest. Cache hits are
reverified, including offline mode. Missing or corrupt offline data fails.
Declared per-file and aggregate byte budgets are checked before access. JSONL
parsing additionally enforces row and byte budgets, preserves file order and
rejects empty/malformed lines. Other formats and adapters remain follow-up work;
unknown formats must not be interpreted as scripts.

HF_TOKEN is used only as an HTTP authorization header. Never put credentials,
acceptance tokens or raw real-user payloads into provenance or evidence.

## Source schemas and runtime boundary

`public_dataset::IngestionRecord` and `IngestionBody` are rich source-stage types:
they retain source-file lineage and prompt, document/question, session or structural
trace representations. They are not runtime records. Only
[`canonical_input::CanonicalRecord` and `CanonicalBody`](CANONICAL_INPUTS.md)
use the canonical names; that document contains the exact final runtime definition.
Do not serialize source metadata into user text or substitute ingestion JSON for
runtime JSONL. Source-row identity remains separate from target identity.
Answers/gold patches must remain outside request bodies. Source rows and target
counts are different units. Sessions explicitly declare recorded versus
generated history; recorded assistant text cannot certify future live reuse.
Tool calls are inert identifiers, not instructions for the ingestion process.

Structural traces declare timestamp units, optional source block size/hashes,
input/output lengths and post-template status. Missing hashes mean overlap is
unknown. Lengths or session IDs alone do not establish prefix identity. Source
hashes cannot be presented as recovered original text or physical cache hits.

### Checked ingestion-to-runtime conversion

Use `ingestion.into_canonical(source_envelope)?`, equivalently the checked
`TryFrom<(IngestionRecord, DatasetEnvelopeRef)> for CanonicalRecord` implementation:

```rust
let runtime: canonical_input::CanonicalRecord = (ingestion, source_envelope).try_into()?;
```

The source envelope is mandatory conversion context because an ingestion target
does not own enough metadata to construct it. Its file list must contain the
record's source-file digest; missing source IDs/row IDs/target IDs fail, not become
fabricated defaults. Envelope/digest and request-boundary validation also apply.
Unknown optional provenance stays unknown; structural conversion is not clearance.

The conversion preserves record/source-row/session/group/trace identities, maps
source output_budget to expected_output_tokens, and checks u64 source turn/output
integers against u32 without narrowing. Unknown arrival remains None. Prompt stays
Prompt with exact text. DocumentQuestion becomes a one-user-message Chat using
`document + "\n\nQuestion:\n" + question`; empty fields fail. Recorded sessions
preserve typed text roles and validate causality, matching session identity and
request boundary. No future assistant answer is appended. Generated history must
first be materialized by the session runner: recorded references cannot stand in
for generated replies. Legacy ingestion tool IDs without function metadata cannot
be upgraded by inventing functions/arguments; use a complete typed tool adapter.
Structural traces fail this generic text conversion until an explicit measured
hash-to-text renderer supplies a request. No text, arrival or reuse is guessed.

Tests cover Prompt/Chat mapping, lineage/envelope preservation, integer boundaries
and overflow, missing identities/provenance, session mismatch, generated-history
and incomplete-tool rejection, and structural traces without original text.
Runtime payload and source-row IDs are not default-public evidence; apply the
source's redistribution/privacy policy before publishing either.

## Validation manifest and report integration

`ValidationManifest` is the data-side sub-contract for study reports.
The JSON Schema is [kv-dataset-manifest.v1](../schemas/kv-dataset-manifest.v1.json).
Report consumers use only `datasets[]` envelopes with exactly these fields:
`source_id`, `resolved_revision`, `config`, `split`,
`files[{path,sha256,bytes}]`, `adapter_id`, `adapter_version`, `license_spdx`,
`validation_manifest{schema_version,sha256}`. `envelope()` constructs this object.
The validation digest hashes compact `serde_json` serialization of the manifest,
with struct field declaration order and ordered maps, excluding any trailing
newline. It is not a hash of pretty-printed JSON. The schema defines this
report envelope under `$defs/envelope`. Unknown license is JSON null, never an
implicit permissive license. JSON uses
`schema_version: "kv-dataset-manifest.v1"`. It includes origin, rights policy, mapping, transformation,
adapter/generator/tokenizer/template versions, corpus-count evidence status,
selected-row/target counts, selection/body-order digests, named check statuses,
exclusions and limitations. The deterministic fingerprint includes every field;
a changed source digest, adapter, tokenizer or template invalidates it. Ordered
maps make field-map/check serialization stable. Arrays retain meaningful order.

`Evidence` distinguishes `measured`, `publisher` and `unknown`. A failed viewer
is not a measured zero-row corpus. Selection hashes do not replace source-file
recounts. A convenience-sample distribution does not certify a corpus. The
human-readable `summary()` deliberately omits payloads and source-row identifiers.

Rights checks fail closed for denial, review-required, unknown license, missing
license-text digest, unresolved terms, absent privacy policy, or an unreviewed
gate. Component/row terms are represented separately from the dataset license.
`redistribution_allowed: false` does not prohibit local reviewed ingestion but
must prohibit downstream raw publication. Public release or hashing is not
privacy clearance. Report exporters must preserve these restrictions.

Record validation checks source-file linkage, nonempty payloads, typed tool
call/result pairing, declared trace block sizes and partial-block hash counts.
This foundation does not yet certify request-boundary causality, answer leakage, rendered
token distributions or source-specific mappings. Those checks must be attached
by adapters and the rendered-token validator before a profile is called validated.
The standalone canonical types are not yet consumed by either runner.

## Offline tests

`cargo test --lib public_dataset` uses only synthetic inline fixtures, under the
repository license; no Hub access, gated content or third-party payload is
required. It checks immutable pins, path validation, cache/offline corruption,
size limits, truncation/checksums, Unicode/order/row budgets, rights gates,
unknown roles and fingerprint invalidation. Live access is not a CI requirement.
