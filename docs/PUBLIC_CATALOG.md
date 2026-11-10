<!-- Copyright (c) 2026 Metrum AI, Inc. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Optional pinned document/code catalog

`public_catalog::entries` is offline discovery metadata, NOT blanket ingestion,
component-content rights, independent privacy clearance or corpus certification.
Every source is optional. A candidate's card license never becomes an automatic
`RightsPolicy::Allowed`. No dataset scripts, source repository tools or ingested
calls run. No catalog lookup downloads data.

| Key | Source at immutable revision | Support and caveats |
|---|---|---|
| loogle-longdep-qa | bigai-nlco/LooGLE @ 4b50b4cb9333b48b3f7ddfd307c1d13369be5d1e | Initial document mapping; CC-BY-SA-4.0 card, underlying-content review, attribution/share-alike mandatory |
| quality | tasksource/QuALITY @ 4892b065e6a40f44fa61bc406df4c35fe91e9290 | Review-only; per-article/jurisdiction terms, repeated rows not independent documents |
| narrativeqa | deepmind/narrativeqa @ 2e643e7363944af1c33a652d1c87320d0871c4e4 | Review-only; Apache card is not blanket story/script rights, count discrepancy needs scoped recount |
| longbench-v1 | zai-org/LongBench @ 5e628be450b7e67fb7ae6e201bd6d8f7056f7672 | Review-only; English words/Chinese characters are not tokenizer tokens, component rights unresolved |
| longbench-v2 | zai-org/LongBench-v2 @ 2b48e494f2c7a2f0af81aae178e05c7e1dde0fe9 | Review-only; extreme contexts need service-window checks, no silent clipping |
| infinitebench | xinrongzhang2022/InfiniteBench @ 90f0394333616266d9fe85824ceaf505093cbaa5 | Review-only; books/code components and count scope unresolved, empty viewer is not zero corpus |
| leval | L4NLP/LEval @ 43b9dbf06c239360b183c76cb5d90ebe4f5e3070 | Review-only; GPL card, document and question counts differ, nested mapping needs validation |
| swe-bench-verified | SWE-bench/SWE-bench_Verified @ 78f471bf655a3137b2e8a75af1501690ec009ec3 | Extraction-required; task issue/base commit is not a full repository context or agent trajectory |

These pins and card hints were checked against anonymous revision metadata and
pinned READMEs on 2026-11-07. Every resolved revision matched exactly. README byte
SHA-256 values are below; API metadata/card licenses remain publisher declarations,
not independent rights clearance or corpus evidence. No corpus file was downloaded
or recounted for this catalog check.

| Key | Pinned README SHA-256 |
|---|---|
| loogle-longdep-qa | a2dbed23d30caf6851e5159a07bb7f041405f9c7667a8306959d8b37e0792bf8 |
| quality | 1f3881c57a3b8650591e300609ba4f511d98ff903008ee1042cb2a3486ae97fb |
| narrativeqa | 6bb0cf93487ba1c63b7039892958ea6f23d07513fa2ad9b16c73ce813bbfa3d1 |
| longbench-v1 | 5c648f2d0f5f1953f9361fdd3dcda23077f103b4028599095a31f03b74fe6458 |
| longbench-v2 | 9fdd1a3ebe86507253c124a18e9f78c898ce6341c12990af17ab868b8f600c35 |
| infinitebench | b79017fb39a6a7feaa4b01d90ad69e9e3de74b8ab3e1b6930715b5e333602551 |
| leval | ab6f16869dc7325336e896786d111708f43c0440fb765945a9924e0578bb5127 |
| swe-bench-verified | 923e6b481ff75c709737251e602bdc311a9be49235b5c20107366747f5640fe4 |

Resolve each README as `https://huggingface.co/datasets/{source_id}/raw/{revision}/README.md`
using the source/revision table above. These are source metadata, not evidence of a
new full-file download/recount. No source count is fabricated from viewer failure.
The LooGLE file pin is documented with license/attribution and fixture limits in
[PUBLIC_DOCUMENTS.md](PUBLIC_DOCUMENTS.md). Its small converted excerpt is not a
full document/corpus benchmark and remains excluded from crate publication.

## CLI discovery and explicit profile gate

```sh
metrum-ai-bench-cli-prompts --quiet catalog
```

Catalog mode reserves stdout for JSON and sends its one-line identity to stderr;
legacy commands retain their banner behavior. This prints `optional-public-catalog.v1`
JSON with `automatic_access_granted=false`
and `source_checks_run=false`. It does not open the Hub or evaluate source rights.
Shape: [optional-public-catalog.v1](../schemas/optional-public-catalog.v1.json).

For an explicitly reviewed supported profile:

```sh
metrum-ai-bench-cli-prompts dataset --job loogle-job.json --offline \
  --catalog-key loogle-longdep-qa --rights-review review.json \
  --output private-inputs.jsonl --report profile.json \
  --consolidation-report consolidation.json
```

`review.json` ([public-catalog-review.v1 schema](../schemas/public-catalog-review.v1.json))
has `schema_version=public-catalog-review.v1`, `origin_sha256`,
`selected_ids_sha256`, `inventory_complete`, `components` (private label → actual
RightsPolicy) and `counts` (scoped count observations, possibly empty). Obtain the
two bindings from the metadata consolidation of a previously authorized profile;
review its private selection/components. Do not invent bindings or set completeness
merely to make a gate pass. A source or selected-ID change invalidates the review.
An empty component map is permitted only with an explicit complete source-only
inventory and no known component terms omitted. This is a recorded policy decision,
not independent clearance. The bounded review loader accepts no unknown fields.

All three output paths must be new and distinct. Generic jobs can also request a
standalone consolidation without catalog mode; missing component inventory then
remains Unknown, not implicitly cleared.

## Library profile gate

`public_catalog::document_mapping` provides only the initial LooGLE field mapping;
unsupported sources fail instead of guessing. It preserves source row identity,
groups by document ID plus validated content hash, puts document before question,
and leaves answers/patches out of mapped requests. Structural field exclusion is
not a semantic answer-leakage guarantee. No retrieval, truncation or service-window
adaptation is implied.

`public_catalog::build` requires a complete explicit job plus reviewed policies.
It checks catalog source/revision/path/bytes/SHA and the reviewed field mapping;
requires pinned renderer/tokenizer configuration; and requires an explicit
component inventory with authorized policies. Source/license/rights gates apply
before ingestion. It reuses `public_profile::build` and #422; it does not implement
another selector. Only fresh, bound rendered evidence and declared component
coverage can satisfy the catalog profile gate. Contradictory scoped counts block
profile acceptance until resolved. Missing corpus evidence stays Unknown.

The return includes private canonical inputs and metadata reports; it does not
publish payloads or issue traffic. Underlying policies and selected-row review
remain operator-declared, not independent legal/privacy clearance. Source review
labels/notes remain private; the [consolidation](VALIDATION_CONSOLIDATION.md) emits
only digests/counts and explicit status. Runtime body/history/nonce transformations
require fresh final token checks. Logical block-aligned ceilings are not physical
hits, eviction, offload, tier participation or saved prefill.

## Code-context boundary

SWE-bench requests need separately versioned/hashes repository extraction at the
pinned task base commit, target repository license review, explicit retrieval/
ordering/truncation policy and gold patch/test exclusion. The task's problem
statement alone is not a full-code context. This catalog does not manufacture
repository contents or execute setup/build/test scripts. Keeping the entry
extraction-required is deliberate, not a successful code adapter claim.
