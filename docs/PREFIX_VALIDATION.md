<!-- Copyright (c) 2026 Metrum AI, Inc. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Rendered-token selection and validation

The `prefix_validation` library adds opt-in constraints independently of source-row
repeat limits. Without constraints, `select` returns the first requested N records
in candidate order. `content_unique` rejects equal rendered body digests **or**
equal token sequences, even if row/record IDs differ. `max_pair_lcp` rejects any
selected pair whose exact leading-token match exceeds the limit. All chat roots,
special tokens, hints and nonces count; different document suffixes do not create
prefix-disjoint inputs. Impossible requested counts fail before traffic with
requested/achieved/candidate counts. The legacy ISL/OSL CLI selector is unchanged;
this first library boundary is not yet wired into its flags or either runner.

## Exact rendering boundary

Render all messages using the chosen pinned template before validation. With the
`tokenizer` feature, `tokenize_rendered` verifies the local tokenizer artifact and
template artifact SHA-256, then encodes the final body with the declared
`add_special_tokens` setting. It does not execute arbitrary template code or
fetch anything. The caller is responsible for actually rendering that template;
the artifact check alone does not attest correct rendering. The rendering identity
records tokenizer ID/40-hex revision, both artifact hashes, generation-prompt and
special-token settings, hint/nonce inclusion, and renderer version. Fixture token
vectors may be supplied directly to the validator but do not independently prove
that a particular real tokenizer produced them.

Supplied input targets remain separate from measured lengths. Token IDs, not
source characters or ordinals, determine overlap; BPE merges at prefix/suffix
boundaries can change the leading token. The feature-enabled local BPE fixture
checks this without any network or downloaded tokenizer. Synthetic token-family
generators must additionally check decode/re-encode drift before serving.

## Validation output

`validate` exhaustively compares each selected pair and each selected/warmup pair.
An explicit `max_pairs` budget fails rather than silently sampling pairs. Output
contains selected length distributions, within-group/across-group pairwise LCP,
warmup LCP, exact-body and exact-token duplicate-pair counts, and the best-prefix
reuse ceiling in effective order. Group-less pairs are across-group. Distributions
report n/min/p50/p95/max, using nearest-rank percentiles; empty n yields nulls.
Small-n percentiles are sample statistics, not corpus certification.

The block ceiling floors the longest match against **earlier** selected records
to whole cache blocks. Partial final blocks are excluded. It assumes infinite
residency of earlier prefixes, no eviction, and does not include warmup residency.
It is not observed saved tokens, hit rate, offload participation or performance.
A warmup threshold can fail any cross-overlap above the allowed root length.

`body_order_sha256` hashes compact JSON serialization of the ordered rendered
record array, including supplied targets and lineage. This is a validation/profile
fingerprint, not the study report's digest of final serialized wire request bodies.
Changing tokenization, bodies, order or metadata changes this fingerprint; the
rendering identity must also be retained alongside it. Full profile validation
fingerprints should hash the entire `OverlapValidation` artifact.

Public evidence should export aggregates/digests, not input tokens, text, tool
arguments or identifiers from real users. Dataset rights/privacy checks remain
owned by [the dataset manifest](PUBLIC_DATASETS.md).
