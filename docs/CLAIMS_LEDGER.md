<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Claims ledger

Status: Approved September 21, 2026. Owner: CEO.

Repository-scope ledger of public claims about Metrum AI Bench CLI. This
ledger tracks claim text, provenance, labeling, and evidence for material
claims made in this repository.

Website, blog, social-media, and product-deck claims are outside this ledger's
scope and are tracked separately.

## Labels

| Label | Meaning |
|-------|---------|
| Measured | Observed from a harness run with required manifest |
| Historical measurement | Observed before the manifest requirement; retained as provenance but not citable as a Metrum AI Bench CLI result |
| Verified-in-code | Asserted by repository code or tests |
| Modelled | Derived from a model or estimate, not a direct run |
| Roadmap | Planned; not a present capability claim |
| Third-party public page | Claim sourced from an external public page |

## Repository claims

| Claim | Source | Label | Evidence |
|-------|--------|-------|----------|
| Metrum AI Bench CLI provides load and performance measurement for OpenAI-compatible LLM, VLM, ASR, and image-generation endpoints | `README.md` and `docs/CLI.md` | Verified-in-code | Shipped modality subcommands and their generated CLI reference |
| Landscape statements compare the documented capabilities of other inference-measurement tools with Metrum AI Bench CLI | `docs/COMPARISON.md` | Third-party public page | Cited public project pages and documentation reviewed in September 2026; no side-by-side run is claimed |
| The checked-in smoke matrix reports NVIDIA-only campaign measurements made with `1.0.0-rc.1` | `docs/SMOKE_RESULTS.md` | Historical measurement | Campaign `matrix-20260915-195537`; the table remains not citable until it is regenerated from `summary.v3` or `campaign.sh report` with a required manifest |
| Documented limitations describe client-side measurement boundaries and output-schema behavior | `docs/LIMITATIONS.md` and `docs/OUTPUT_SCHEMA.md` | Verified-in-code | Harness behavior, tests, and the published output schema |

## Maintenance

- Add a row when a new material public claim is introduced in this repository.
- Update evidence when a claim is re-measured or re-verified.
- Do not treat Historical measurement, Roadmap, or Modelled rows as Measured.
- Keep claims outside the repository in their channel-specific ledgers.
