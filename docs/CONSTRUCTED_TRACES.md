<!-- Copyright (c) 2026 Metrum AI, Inc. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Declared structural hash-to-constructed-text bridge

`trace_text::construct_jsonl` (tokenizer feature) constructs **new ASCII literal
content** from a verified source topology. It never recovers original text or
decodes source hashes as original token IDs. This is not semantic replay or an
observed serving-cache workload. No traffic or ingested source code is executed.

## Source and tokenizer trust boundary

The function receives pinned source bytes/origin/file, reviewed rights policy,
declared mapping and source limits. It invokes the verified JSONL adapter itself;
a deserialized `SourceTrace` is not implicitly trusted by this API. Original row
IDs/positions, source time, lengths, parent metadata and source fingerprint remain
private source provenance. Rights/access gates are not overridden by construction.

The local tokenizer is pinned by revision label and exact byte SHA-256. File reads
are bounded. Padding/truncation configurations are rejected. No remote tokenizer,
source script, Python plugin or decoder is fetched/executed. The operator supplies
1–64 reviewed ASCII alphanumeric literals, at most 128 bytes each. No source
identifier or raw hash is interpolated into generated text.

## Construction and limitations

- Require known full-prefix nodes covering exactly `floor(source_input/B)` for
  each selected row, where B is the declared source block size. Unknown or
  incomplete full-prefix coverage fails closed, rather than inventing known nodes
  from lengths. Empty source inputs are rejected.
- Select bounded literal segments deterministically from seed, source
  normalization fingerprint and logical node identity. Reused nodes reuse the
  same segment along the same parent path. Different sibling full-node blocks
  must tokenize differently. Failed exact lengths, boundary merges or collisions
  retry only within explicit segment-attempt limits, then fail.
- Tails are newly seeded literals of the remaining token length. They are NOT
  reconstructed source tails; no original partial-tail reuse claim follows.
- Final raw-prompt tokenization must preserve all constructed full-node boundaries
  and exact source input length. Tokenizer work is bounded by call and aggregate
  input-byte budgets, in addition to per-candidate bytes, output tokens/bytes,
  rows and source expansion limits. No expensive decoder expansion is needed.
- A configured alphabet is not guaranteed representable under every tokenizer.
  Unsupported/infeasible configurations fail; there is no truncation, silent
  approximation, retry-until-success or relaxed assertion fallback.

Sharing is checked within the verified selected artifact/normalization domain.
Opaque node IDs and generated text are not stable cross-domain identities when
selection/mapping/source content changes. Distribution fidelity and original
semantic equivalence are not claimed.

## Evidence and runtime boundary

Output stores newly constructed text, raw token IDs, preserved source lineage,
known source full-node IDs and constructed tail lengths. Full artifacts are
**private by default**, including identifiers and operator-provided literals.
`constructed_raw_prefixes_validated` only describes the generated raw prompts.
It does not establish final chat-template overlap, issued-body identity or physical
cache reuse. `source_original_text_recovered` stays false, rendered overlap remains
Unknown, and physical cache evidence remains unavailable.

The private [artifact schema](../schemas/constructed-source-trace.v1.json) is
shape validation, not constructor revalidation or rights approval. The
`trace_text::metadata` helper emits only counts/digests and explicitly labels the
constructor-only validation scope; it is not a substitute for revalidation.

## Opt-in offline probe

After separately verifying the source cache with `trace_source_probe`, the
`trace_construct_probe` example selects exactly the first four pinned Qwen rows.
It is always offline and requires explicit source-policy acknowledgement:

```sh
cargo run --locked --features tokenizer --example trace_construct_probe -- \
  --accept-source-terms ../trace-cache ./tokenizer.json ./construction-job.json
```

The job JSON is `{"$comment":"Copyright/description", "config":{...}}`, where
`config` supplies every `TextConstructionConfig` field from the artifact schema.
Use a reviewed local tokenizer, its exact digest/revision, reviewed literal
alphabet and explicit limits. A synthetic tokenizer is valid for a structural
construction smoke test, not a replacement for final target-model rendering.
The example emits only the metadata summary and does not save full artifacts.
Failure to meet work/resource/exact-boundary limits is a failure, not a source
fallback. The source cache and operator inputs should remain outside the repo.

## Explicit candidate projection

`trace_candidates::construct_candidates_jsonl` calls the verified constructor and
returns the private construction, a truthful `ValidationManifest`, and checked
**constructed Prompt candidates** using the unchanged canonical converter. It
does not accept deserialized construction claims as fresh evidence. The adapter
ID `constructed-source-trace` and manifest transformations distinguish new content
from original text. Stable constructed record IDs bind file/source-row/construction
digests; publisher source-row identity remains separate.

Reference source output lengths remain private metadata, NOT requested output
targets. Arrival/session/group/turn fields stay unset; source graph/history/time
remain in the private construction and are not invented flat runtime semantics.
The trace ID explicitly names the constructed artifact. Raw-body order, selected
IDs, construction/tokenizer fingerprints and actual source rights are retained in
the manifest. Corpus/rendered/physical evidence stays Unknown.

Candidates are **not automatically issued requests or causal/session replay**.
Runtime must explicitly select request points, interpret declared source graph
semantics, apply any source-relative-time policy, and validate final rendered/
issued bodies. The library does not change canonical types or the runner schema.
