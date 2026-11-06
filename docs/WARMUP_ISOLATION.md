<!-- Copyright (c) 2026 Metrum AI, Inc. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Offline warmup isolation interface

`metrum_ai_bench::token_overlap::validate_prefix_disjoint(warmup, measured,
block_size_tokens) -> OverlapReport` is a pure offline check. It performs no file
access, networking, tokenization, template execution, reset or request issuance.
It does not infer isolation from file names, request IDs or source rows.

## Input boundary

`RenderedBody` contains exactly `request_id`, `phase` (`Warmup` or `Measured`),
`body_sha256`, `tokenizer_revision`, `chat_template_sha256`, `token_ids`.
The caller owns rendering. Body SHA-256 must hash **exact final HTTP JSON body
bytes**, after all order/mutation/history transformations, not a prompt string,
headers, pretty-printed reconstruction or supplied token target. Token IDs must
represent fully rendered chat input under the declared pinned tokenizer/template.
The function cannot independently attest either claim without original bytes or
tokenizer artifacts; callers must retain that provenance.

Tokenizer revision must be 40 hex characters, template/body hashes 64 hex, and
request IDs/tokens nonempty. Every record must match the expected phase and the
same tokenizer/template identity. Empty sets, zero block size, invalid inputs or
any mismatched rendering identity return `Unknown`, never `Pass`. On Unknown,
maxima are placeholders, not measurements, and identity fields are empty.

## Output contract

`schema_version` is `kv-token-overlap.v1`. The report contains:

- `warmup_body_digests_sha256`, `measured_body_digests_sha256`: SHA-256 over UTF-8
  compact JSON arrays of lexicographically sorted `body_sha256` strings, retaining
  duplicates. Array order changes do not affect these two evidence digests.
- `max_shared_prefix_tokens`: maximum exact leading-token match across the sets.
- `max_shared_full_blocks`: floor(max_shared_prefix_tokens / block_size_tokens).
  Partial shared blocks do not count as full reusable blocks.
- `pairs_over_threshold`: bounded `(warmup_request_id, measured_request_id,
  shared_prefix_tokens)` triples exceeding the permitted **full-block** threshold.
  Stable order is measured input order, then warmup token-lexicographic order
  with original warmup position breaking ties. Duplicate records count separately.
- `pairs_over_threshold_total`: exact offending pair count including duplicates;
  `null` on Unknown, never an inferred zero.
- `pairs_over_threshold_truncated`: true when the stored list omits offending
  pairs; false on Unknown (the total remains null and the verdict is authoritative).
- `tokenizer_revision`, `chat_template_sha256`, and `verdict` (`Pass`, `Fail`,
  `Unknown`). Enum JSON spellings preserve these exact capitalized names.

Default threshold is zero: any shared full block means `Fail`.
`validate_prefix_disjoint_with_threshold` allows a configured full-block threshold;
callers must record any non-default threshold. IDs remain non-sensitive stable
opaque request identities. Report tuples should not expose real-user identifiers.

Both existing functions retain their signatures and store at most 1,000 pairs by
default. `validate_prefix_disjoint_with_limits(warmup, measured, block_size_tokens,
allowed_shared_full_blocks, max_reported_pairs)` permits an explicit cap, including
zero. A zero cap can return Fail with an empty stored list: consult the verdict
and total. Caps never change exact maxima, totals, digest multiplicity or verdict.
The two fields extend the unreleased `kv-token-overlap.v1` in place; schema and
consumer mappings must account for them before release.

A compressed warmup token trie references existing token spans rather than
allocating a node per token. Descendant intervals preserve duplicate counts.
Measured bodies are walked once; extra per-pair LCP work is limited to stored
pairs. Warmup lexicographic indexing and construction are separate setup work.
No quadratic scan of every warmup/measured pair is performed for totals/maxima.

## Issuance/report integration

The caller invokes the check **before measured traffic**. Fail closed on `Fail`
or `Unknown` unless a deliberate operator override is enabled. Record the report,
both evidence digests, threshold and any override in the study manifest. This
module does not implement the override or runner lifecycle. A matching digest
binds the provided body list; it is not proof of kernel warmup, cache reset,
residency, physical hits or offload performance. Fully disjoint validation input
does not establish that different bodies were actually issued.

Offline unit fixtures cover mismatched tokenizers/templates, invalid phases and
pins, identical prefixes with different request IDs, partial blocks, configurable
thresholds, sorted digest multiplicity, brute-force parity across branch/prefix/
duplicate cases, and caps including zero. A CPU-only regression validates 2,000
warmup by 2,000 measured bodies of 8,000 tokens within 30 seconds, checks all
4,000,000 offending pairs are counted while only 1,000 are stored, and verifies
long duplicate edges do not allocate per token. This is an algorithm regression,
not a serving-performance claim. No credentials or Hub access are used.
