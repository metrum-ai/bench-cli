<!-- Copyright (c) 2026 Metrum AI, Inc. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Cross-source provenance/rights consolidation

`validation_consolidation` extends the existing `ValidationManifest` and #422
rendered-token artifact. It is a metadata-only consolidation layer, **not another
selector**, fresh source-byte verification, independent legal/privacy assessment,
final runtime request-body measurement, or physical-cache certification.
Canonical inputs, manifest fields and study-report dataset envelopes are unchanged.

## CLI and logical job identity

```sh
metrum-ai-bench-cli-prompts dataset --job job.json --offline \
  --output private-inputs.jsonl --report profile.json \
  --consolidation-report consolidation.json
```

Outputs must be new distinct paths, including aliases through parent directories.
Canonical payload stays separate from metadata; files are never overwritten.
The consolidation JSON contains its human-readable summary, also printed by CLI.

The existing manifest transformation list binds `public-logical-job.v1` to the
entire logical declaration: source/rights, full mapping (including requested
output budget), formats, limits, selection/count/block settings, renderer identity,
special-token context and rendering limits. Even unused rendering context or
output-budget changes invalidate validation provenance when prompt text happens
to stay identical. Local filesystem paths are excluded: moving identical pinned
artifacts is not a semantic change. No canonical/manifest/report fields changed.

## Evidence boundaries

The report fingerprints the exact existing manifest, source origin, rights policy,
selected IDs, body order and component-review inventory. It exports JSON plus a
human summary without raw row identifiers, texts, component labels or review notes.
Raw reviewed policies/source artifacts remain separate and private by default.
A digest is not independent privacy clearance or permission to publish payloads.

A supplied #422 overlap artifact must match the manifest's exact artifact SHA,
tokenizer revision, template digest, target/sample count and positive block size.
It is reused rather than recalculating selection or LCP. Missing rendered evidence
stays Unknown. The measured scope is the selected convenience sample, not corpus
quantiles or final issued-body tokens. Subsequent body/history/template mutation
requires new validation; matching an old serialized artifact is not remeasurement.

Declared access-policy authorization is reported separately from independent
privacy/legal clearance. Denied is a failure; review-required, missing license
text digest or unresolved rights remains Unknown. Public release never implies
clearance. Independent privacy, issued-body and physical cache checks remain
Unknown in this source-stage report.

## Component and row terms

Review inventories bind private component/row labels through
`review_scope_sha256` to the exact source origin and selected-ID fingerprint.
Unknown/omitted/duplicate/unbound scopes cannot silently satisfy declared coverage.
The caller must explicitly declare inventory completeness; a review of a few
convenience rows is not an automatically complete corpus rights inventory.
`ComponentReview::bind` records exact origin and selected-ID bindings; stale
reviews cannot silently transfer to another selection. Known component terms in
the base policy must be represented, even if the caller tries to declare an empty
complete inventory. Card/database/code terms are not
blanket component-content rights. These are recorded policy reviews, not a legal
opinion or independent content-clearance audit.

## Counts and contradictions

Counts retain their original `measured`, `publisher` or `unknown` status. Comparison
requires the same exact scope digest AND counting unit (source rows, documents or
derived targets). A contradiction is flagged with observation indices, without
printing source identifiers. Different units/scopes are not silently reconciled.
Empty, singleton or unknown evidence cannot establish count consistency. A failed
viewer has `completeness=unavailable` and must be represented as Unknown; a known
zero with unavailable status is rejected, not interpreted as zero corpus rows.
Partial viewer claims remain partial even when their values happen to agree.
Every observation explicitly declares `claim=scope_total` or
`claim=observed_subset`, separately from metadata completeness. Observed subsets
are never compared as full totals and cannot produce consistency Pass. Conversely,
incomplete viewer metadata can still *claim* a total; conflicting total claims
with the same scope/unit remain visible discrepancies (for example 32,747 versus
14,650). Incomplete metadata never independently establishes consistency Pass.
This distinction preserves contradictory publisher/viewer claims without falsely
comparing a known subset size against a corpus total. A viewer page's row count
must be `observed_subset` unless total semantics are explicit; an unavailable or
empty failed viewer is Unknown, not a zero total.

The manifest's corpus status is retained, never inferred from sample size or
physical lines. Even an agreeing publisher/viewer pair is only metadata consistency,
not a source-file recount. Full-file recounts still apply only to the explicitly
verified file inventory/config/split, not automatically to every dataset revision.

Semantic answer/patch leakage, source role/tool/request-boundary interpretation,
output-budget distributions, document ordering/truncation, trace units/hash
semantics, desired/achieved order and live-versus-recorded history remain explicit
Unknown checks in this generic consolidation unless supported by a separate
source-specific validation. Canonical shape or a field-name exclusion is not a
semantic/history/serving proof.

JSON shape: [source-validation-consolidation.v1](../schemas/source-validation-consolidation.v1.json).
Shape validation and deserialization are not source remeasurement or rights approval.

## Optional catalog relationship

Catalog discovery is not authorization. Source/profile eligibility must pass the
same per-run rights and rendered-token gates; review-only/extraction-required
candidates cannot become usable merely because a card says Apache or a viewer
loads. Source count contradictions remain visible until resolved with scoped
source-file evidence. Unsupported mappings and extreme windows fail rather than
clip or import dataset scripts.
