<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Claims ledger

Status: DRAFT — pending counsel review (C3). Owner: CEO.

Repo-scope ledger of public claims about Metrum AI Bench. Use this template to
track claim text, provenance, labeling, and evidence. Re-verify rows after
`rc5/docs` merges land.

## Labels

| Label | Meaning |
|-------|---------|
| Measured | Observed from a harness run with required manifest |
| Verified-in-code | Asserted by repository code / tests |
| Modelled | Derived from a model or estimate, not a direct run |
| Roadmap | Planned; not a present capability claim |
| Third-party public page | Claim sourced from an external public page |

## Repository claims

| Claim | Source file:line | Label | Evidence |
|-------|------------------|-------|----------|
| README product / capability claims | `README.md:*` | *re-verify after rc5/docs merges* | *re-verify after rc5/docs merges* |
| COMPARISON positioning claims | `docs/COMPARISON.md:*` | *re-verify after rc5/docs merges* | *re-verify after rc5/docs merges* |
| SMOKE results summary claims | `docs/SMOKE_RESULTS.md:*` | *re-verify after rc5/docs merges* | *re-verify after rc5/docs merges* |
| LIMITATIONS statements | `docs/LIMITATIONS.md:*` (or successor) | *re-verify after rc5/docs merges* | *re-verify after rc5/docs merges* |
| External docs site pages | `external-docs/content/**/*.mdx` (esp. comparison, limitations, results-publication) | *re-verify; pages carry draft banners* | *re-verify after rc5/docs merges* |

## Website / post / deck

| Claim | Source | Label | Evidence |
|-------|--------|-------|----------|
| TODO(CEO) — inventory website claims | metrum.ai (and related) | TODO(CEO) | TODO(CEO) |
| OSS docs site (`docs.metrum.ai` / `external-docs/`) | comparison, limitations, results-publication, intro draft banners | *pending counsel / ledger re-verify* | Draft banners added; do not publish as counsel-cleared |
| TODO(CEO) — inventory blog / social posts | public posts | TODO(CEO) | TODO(CEO) |
| TODO(CEO) — inventory product deck claims | decks / presentations | TODO(CEO) | TODO(CEO) |

## Maintenance

- Add a row when a new public claim is introduced in-repo or on owned channels.
- Update Evidence when a claim is re-measured or re-verified.
- Do not treat Roadmap or Modelled rows as Measured.
