<!-- Copyright (c) 2026 Metrum AI, Inc. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Public document/question inputs

`public_adapters::documents` is the initial declarative document adapter on the
[dataset contract](PUBLIC_DATASETS.md). It maps one question per row or an array
of questions per document into IngestionRecord/IngestionBody::DocumentQuestion.
The [dataset command](PUBLIC_DATASET_CLI.md) exports them through the checked
[ingestion-to-runtime conversion](PUBLIC_DATASETS.md) as the final CanonicalRecord
with source_row_id and body: CanonicalBody::Chat. Runtime consumption remains a
separate interface. The optional catalog stays deferred until rights and rendered
token gates are complete.

## Pinned first fixture

Source: [bigai-nlco/LooGLE](https://huggingface.co/datasets/bigai-nlco/LooGLE/tree/4b50b4cb9333b48b3f7ddfd307c1d13369be5d1e),
revision `4b50b4cb9333b48b3f7ddfd307c1d13369be5d1e`, file
`data/longdep_qa.jsonl`, 134,979,019 bytes, SHA-256
`6c047bc6e904e5fd632e44a2f2624d75e85fe0f871c6b74bcfffa918bf2f3624`.
The size/digest came from pinned Hub LFS metadata; a full-file recount/download
was not performed for this fixture. The adapter labels this file mapping
`config=longdep_qa`, `split=test`; it does not execute the dataset's loader script.
The pinned card declares **CC-BY-SA-4.0**, not Apache-2.0. Card SHA-256 is
`a2dbed23d30caf6851e5159a07bb7f041405f9c7667a8306959d8b37e0792bf8`;
this is a card digest, **not** a legal-code text digest or blanket rights clearance.

Attribution: Jiaqi Li, Mengmeng Wang, Zilong Zheng and Muhan Zhang, *LooGLE: Can
Long-Context Language Models Understand Long Contexts?* (2023), arXiv:2311.04939.
The source document title for the excerpt is *José Luis Picardo*.

The committed fixture `test-data/public-datasets/loogle-excerpt.v1.json` contains
only the first 160 Unicode characters of the first row's historical biography,
its public question/IDs/title, and a conversion manifest. Answers/evidence/task
were omitted. Original first-line digest is recorded separately from the full
source-file digest and the converted fixture's own bytes. The changes are explicit;
this fixture cannot represent a full-document length distribution or corpus
profile. Retain attribution and share-alike conditions for the excerpt; the
repository's software license does not relicense source content.

Public availability does not clear underlying component rights or privacy. No
real-user chat, gate bypass, inference or private content is involved in this
fixture. Broader datasets may contain PII, unsafe material or component terms;
local reviewed policy remains mandatory. Tests use an explicitly synthetic
rights-policy object, not independently verified legal clearance. Ingestion
requires the caller's actual license-text hash and recorded policy.

## Mapping and validation

The first mapping uses `id` as source-row identity, `doc_id` as document identity,
`context` as document and `question` as the user question. A nested-question mapping
may instead name an array field (for example an explicitly reviewed instructions
array). Config/split are checked against the mapping before conversion. Fields
are not auto-guessed. Source SHA must be present in the exact origin manifest.

Canonical target ID hashes source/revision/file/row/question-index. Group ID hashes
source/document-ID/document-content-digest, so the same document ID with changed
content cannot masquerade as one reused prefix. Questions on one document share
a group but retain distinct targets. Duplicate target identity fails.

`render_document` emits document first and question last, with an explicit separator.
It performs no truncation, retrieval or service-window clipping. The caller adds
chat framing, validates final lengths/overlap with the pinned tokenizer/template,
and checks service-window feasibility separately. Successful mapping is not token
validation or evidence of cache reuse. A changed template invalidates validation.

Only explicitly selected document/question fields enter request bodies. Known
answer/patch field names are prohibited as request mappings; other source metadata,
answers and patches are not copied. This is a structural leakage guard, not a
semantic guarantee that a source author never embedded an answer inside context.
Final adapter/manifest checks must retain that limitation.

Source row counts and derived question targets are different. Target-count and
aggregate derived-body-byte budgets fail before cloning unbounded repeated
contexts. Unicode is preserved. Empty/missing fields, config/split mismatch,
unknown/denied rights and duplicate identities fail deterministically. No tests
perform Hub requests; the pinned excerpt and synthetic rows are local.
