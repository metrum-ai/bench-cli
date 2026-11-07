<!-- Copyright (c) 2026 Metrum AI, Inc. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Typed public session normalization

`session_inputs::normalize` consumes rows from the shared checksum-verified file
reader and emits **source SessionTranscript values**, not runtime request points.
The session runtime alone chooses request boundaries, executes sequential turns,
and owns reset/warmup lifecycle. A complete source transcript ending in a reference
assistant is not an unconditional model request. CanonicalRecord/CanonicalBody
remain the separate final runtime contract in [CANONICAL_INPUTS.md](CANONICAL_INPUTS.md).

## Sources, revisions and access

The initial schema mappings target these publicly documented sources; file pins,
license-text digests and policy decisions must still be supplied and verified for
actual ingestion. A source name/revision is not a blanket content-rights certificate.

| Source | Revision | Declared license | Mapping/privacy caveat |
| --- | --- | --- | --- |
| HuggingFaceH4/ultrachat_200k | 8049631c405ae6576f93f445c6b8166f76f5505a | MIT card metadata | Synthetic source; preserve prompt_id, messages/role/content; keep train_sft/test_sft and generation splits distinct. No production arrival inference. |
| HuggingFaceH4/mt_bench_prompts | e3a795c5e9a82ee40611c416b8a7786c73198991 | Apache-2.0 card metadata | Raw turns and Parquet prompts are distinct mappings, not silent fallbacks. Prompt-only multi-turn data needs generated history; it supplies no assistant replies. |

These mappings are not an automatic optional catalog. ShareGPT mirrors and other
real-user sources require independent rights/privacy review; this adapter does not
sample or publish their conversations. Gated datasets require exact operator terms
review through the shared rights gate. Unknown rights block normalization. No raw
real-user fixture is committed, no gate is bypassed and no live endpoint is used.

## Mapping and identities

SessionMapping declares adapter ID/version, expected config/split, row identity,
recorded versus generated history mode, optional expected_output_tokens and shape.
Optional `tools_field` retains typed function definitions (name, description,
parameters schema and optional strict flag). Unknown definition fields/types and
duplicate names fail rather than disappearing. `tool_arguments_encoding` defaults
to `serialized_json`; `json_object` explicitly serializes source objects into inert
canonical argument strings. `tool_result_json=true` explicitly serializes non-text
tool results; otherwise non-text content fails. No representation is guessed.
The Chat shape declares messages, role/content fields, an explicit role-label map,
and optional tool_calls/tool_call_id fields. For example, role/value conventions
can map human to User and gpt to Assistant without guessing unknown labels.
The Turns shape selects exactly one declared array field and preserves each user
turn. Missing/empty/unknown-role/multimodal fields fail. No assistant placeholders
are fabricated; multi-turn prompt-only recorded replay is rejected.

Source row identities may be strings or unsigned integers. Opaque session IDs hash
source ID/revision/config/split plus the source-row identity, independently of row order. Duplicate
session identities fail within a normalization batch; callers combining files must
also reject duplicate session IDs across their combined source set. File digest must
belong to the pinned origin. The caller verifies file bytes before parsing: a parsed
Value alone cannot prove a file checksum. Per-batch session/message/serialized-body
limits apply. Full transcripts and raw source-row identifiers remain private by
default, subject to the reviewed source redistribution/privacy policy.

## Tools, history and request boundaries

Typed Message/ToolCall values preserve role, content, call ID, function name,
serialized JSON-object arguments and result linkage. Arguments are inert strings:
never dispatch functions, commands or repository code during ingestion. Unmapped
OpenAI-shaped tool metadata is rejected rather than silently discarded. Tool calls
must be causally matched exactly once before the next conversation message; a source
transcript cannot end with unresolved calls. Assistant-only sources fail.

The adapter validates the complete causal stream without selecting a request point.
A temporary validation-only user suffix permits a source's final reference assistant
while enforcing all ordinary role/tool ordering checks. This suffix is never emitted,
never counted and never sent to a server. The original message sequence is retained
unchanged. The runtime must exclude the current/future reference answer at each
request boundary and keep source references separate from actual generated replies.

Recorded mode is labeled replay. Generated mode retains source references as source
data only; the runtime must construct actual generated history rather than claim
those source assistant tokens will occur. No live-history token reuse is certified
by normalization. Source tool results are recorded data, not live executions. No
arrival time, think time or production timing is inferred from conversation order.

## Validation and safe evidence

`session_inputs::metadata` returns counts, role distribution and an ordered
normalization fingerprint. Raw source IDs, messages and tool arguments are omitted.
Corpus counts, arrivals and live-history overlap remain unknown. The normalization
fingerprint includes mapping/version, mode, source-file digest, row identity and full
message content and retained tool definitions; changed source references or schemas
invalidate it. It is not a final HTTP
body fingerprint, server hit rate, saved tokens or observed cache-tier activity.

Small synthetic UltraChat-shaped and MT-Bench-shaped fixtures test source reference
preservation, exact role mapping, no placeholder invention, numeric IDs, duplicates,
malformed/empty/assistant-only sources, typed tool pairing, explicit history modes,
limits and rights denial. These are schema fixtures, not copied upstream rows or a
corpus certificate. Full generated-history behavior and rendered warmup isolation
require the session runtime and pinned final-body validation.
