<!-- Copyright (c) 2026 Metrum AI, Inc. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Public dataset command

`metrum-ai-bench-cli prompts dataset --job job.json --output canonical.jsonl --report validation.json --offline`
invokes the same command as `metrum-ai-bench-cli-prompts dataset ...`. Existing
ISL/OSL selection flags and legacy string loaders remain unchanged. This command
builds inputs; it does not issue benchmark traffic, execute dataset scripts or
run tool calls. Canonical runtime consumption is a separate runner interface.

`--offline` requires a verified content-addressed cache or a complete explicit
`local_files` map. Without it, reviewed pinned Hub files may download, under the
job's byte budgets. No mirror fallback, script execution, agreement acceptance
or credential persistence occurs. HF_TOKEN is only used by existing Hub headers.
All requested outputs must be new files in existing parent directories; existing
files are never overwritten. Parent-directory aliases are checked before writing,
so a payload path cannot also be selected as a metadata path. A failed
write must be treated as failure even if one output was already installed.

## Scoped consolidation and optional catalog

Add `--consolidation-report consolidation.json` for the standalone metadata-only
[provenance/rights report](VALIDATION_CONSOLIDATION.md). It binds the exact existing
manifest and #422 rendered artifact; missing component reviews, corpus evidence,
final-issued-body and physical evidence stay Unknown. Its human summary is printed
and stored in JSON. All logical job settings, including requested output budget and
rendering context, bind through the existing manifest transformation list; changing
local filesystem locations alone does not alter identity.

`prompts catalog` lists [optional pinned candidates](PUBLIC_CATALOG.md) without
network access, source verification or rights grants. For the initial reviewed
profile, `--catalog-key loogle-longdep-qa` additionally requires `--rights-review`
and `--consolidation-report`. Review JSON is bounded and binds exact source and
selected IDs; stale/incomplete/unknown policies cannot satisfy the profile gate.
Pinned rendering is mandatory in catalog mode. Review-only/code-extraction entries
fail rather than guess context or execute source scripts. Catalog metadata/card
licenses are not legal clearance or serving-window guarantees.

## Job schema

The local JSON job is limited to 256 KiB and uses
`schema_version: "public-dataset-job.v1"`. Unknown fields fail. Required fields:

- `origin`: exact source_id/resolved_revision/config/split/files with digests/bytes.
- `rights`: reviewed policy from [the dataset contract](PUBLIC_DATASETS.md).
- `mapping`: explicit [document/question mapping](PUBLIC_DOCUMENTS.md), including
  adapter ID/version, expected config/split, field names and output budget.
- `formats`: one explicitly declared format per pinned source path.
- `local_files`: empty for Hub/cache, or a complete source-path to local-path map.
  Partial local overrides fail rather than fetching missing files implicitly.
- `limits`: max_file_bytes, max_total_bytes, max_rows, max_targets, max_body_bytes,
  max_pairs. All budgets are explicit; source rows and derived targets differ.
- `count`: null for all derived targets, or the required positive target count.
- `constraints`: content_unique (boolean) and max_pair_lcp (integer or null).
- `block_size_tokens`: declared full-block granularity for overlap validation.
- `rendering`: null for structurally checked/unmeasured inputs, or pinned rendering
  configuration as described below.

The initial command maps documents/questions, not an automatic optional catalog.
It expands nested questions under a derived target budget. Source answers, patches
and unrelated metadata do not enter the request body. Empty/malformed inputs,
config mismatch, source corruption, rights denial and impossible target selection
fail before output. There is no silent truncation or service-window clipping.

### Explicit file formats

Plain format examples: `{"format":"jsonl"}`, `{"format":"json_array"}`,
`{"format":"parquet"}`. Array/JSONL rows must be objects; order is preserved.
Parquet row counts and declared uncompressed row-group sizes are checked before
iteration, with actual row/decoded-JSON byte checks during conversion. Source
bytes/digests are verified before parsing. These parser budgets are not a general
allocator/process sandbox for malicious parser inputs; use reviewed pinned files.

ZIP example:

```json
{"format":"zip","max_entries":32,"members":[
  {"file":{"path":"data/questions.jsonl","sha256":"<64-hex-member-digest>","bytes":1234},"format":"jsonl"}
]}
```

Pin the outer ZIP in origin.files and every selected uncompressed member here.
Selection order is explicit, not directory order. Member lineage/digests enter the
manifest transformation list. Source row IDs are namespaced by member path before
canonical target hashing. No archive path is used for filesystem extraction;
verified members are parsed from anonymous temporary files. Traversal names,
symlinks, duplicate selections, ambiguous paths, unsupported encryption/compression,
wrong sizes/digests and row/byte/entry-budget violations fail. Recursive archives
and executable format detection are unsupported. ZIP support uses MIT-licensed
zip 9.4.1 (declared MSRV 1.88), with only Deflate support enabled beyond stored data.

### Pinned rendering and selection

Rendering requires a binary built with `--features tokenizer`. A rendering object
contains identity (tokenizer_id, tokenizer_revision, tokenizer_sha256,
chat_template_sha256, add_special_tokens, add_generation_prompt,
hint_nonce_included, renderer_version), local tokenizer_path/template_path,
special_tokens bindings, max_artifact_bytes, max_rendered_bytes, max_tokens and
template_fuel. Use renderer_version `minijinja-chat.v2` (tool-context normalization).
See [CHAT_RENDERING.md](CHAT_RENDERING.md). Local artifact paths never enter the
public manifest. Unsupported framing and implicit tokenizer truncation/padding fail.

The fully rendered document/question tokens drive opt-in content uniqueness and
prefix constraints. Counts cannot substitute for real token identity. Selection
is deterministic first-fit in source order; the work/pair budget is explicit.
Selected overlap results include measured lengths, within/across-group LCP and
block-aligned reuse ceilings. See [PREFIX_VALIDATION.md](PREFIX_VALIDATION.md).
Without rendering, token checks remain **Unknown**, not a pass. Requesting token
constraints without a renderer fails. Structural output remains useful for input
preparation, but cannot be advertised as a validated KV-reuse/bust profile.

## Outputs, fingerprints and limitations

`canonical.jsonl` contains [runtime records](CANONICAL_INPUTS.md) with opaque
record/group IDs, preserved source_row_id, body: CanonicalBody::Chat, positive
expected_output_tokens and the exact dataset envelope. The checked conversion is
shared with the canonical runtime contract; it does not silently narrow integers.
It contains raw source payload: keep it local unless the reviewed policy permits
redistribution and privacy review. Do not feed it to a legacy Vec<String> loader
and expect lineage to survive. Runtime must use the canonical loader/input enum.

`validation.json` contains manifest, optional overlap artifact and human summary;
it omits raw prompt text, tool arguments and source row identifiers. Corpus count
stays unknown; an actual selected-file row recount is a separate measured check.
Distinct source rows, expanded targets and selected targets have separate counts.
Convenience-sample percentiles do not certify a corpus.

The manifest's selected_ids_sha256 hashes compact JSON of ordered canonical target
IDs. Here body_order_sha256 hashes compact JSON of ordered document/question text
SHA-256 values. It is **not** the final HTTP body checksum. The overlap fingerprint
also includes rendering identity/actual measurements. References in each runtime
record point to the full validation manifest fingerprint, not to the outer report.
Any subsequent scheduling/history/cache mutation requires rerendering final HTTP
bodies and the runtime warmup-isolation check. Logical overlap is never observed
server hits, saved prefill time, tier participation or offload performance.

Tests use only local synthetic JSON/Parquet/ZIP fixtures and the attributed pinned
LooGLE excerpt. No CI test downloads from the Hub or invokes a serving endpoint.
