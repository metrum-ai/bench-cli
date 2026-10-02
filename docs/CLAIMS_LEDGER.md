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
| Measured | Observed from a harness run |
| Verified-in-code | Asserted by repository code or tests. Tests that run against `dummy-model-server` prove request plumbing and timing, not that a real server accepts the payload. |
| Verified-live (`<campaign id>`, `<date>`) | Observed against a real serving stack in the named campaign or review, with the data log kept. Applies per modality. |
| Modelled | Derived from a model or estimate, not a direct run |
| Roadmap | Planned; not a present capability claim |
| Third-party public page | Claim sourced from an external public page |

## Repository claims

| Claim | Source | Label | Evidence |
|-------|--------|-------|----------|
| Metrum AI Bench CLI provides load and performance measurement for OpenAI-compatible LLM, VLM, ASR, and image-generation endpoints | `README.md` and `docs/CLI.md` | Verified-in-code | Shipped modality subcommands, their generated CLI reference, and e2e tests against `dummy-model-server -strict-media`. Live status is per modality in the next four rows. |
| LLM measurement works against a real serving stack | `README.md` | Verified-live (2026-10-01 launch readiness review, 1x H100; campaign ID not yet recorded in this repository) | Readiness review approved the LLM smoke on 1x H100. Add the campaign ID and data-log location when the review record lands. |
| VLM measurement works against a real serving stack | `README.md` | Verified-live (2026-10-01 launch readiness review, 1x H100; campaign ID not yet recorded in this repository) | Readiness review approved the VLM smoke on 1x H100. Inline `data:` image URLs (fixed after 1.5.2) and the 512x512 fixture have not been run live yet. |
| ASR measurement works against a real serving stack | `README.md`, `docs/ASR.md` | Verified-live, functional only (2026-10-01 launch readiness review) | Transcriptions succeeded, but no run has produced WER/CER against a real server: the review had no reference transcripts, and campaign `matrix-20260915-195537` (2026-09-15) had 0 successful transcriptions (HTTP 400 on every upload). WER becomes claimable once `live-modality-smoke.yml` passes for `asr` with `test-data/asr/` ground truth. |
| Image-generation measurement works against a real serving stack | `README.md` | Verified-in-code only (not verified live) | Never run against a real server. `docs/SMOKE_RESULTS.md` lists imagegen as deferred, and the readiness review blocked it. `scripts/live/serve/imagegen.sh` (vllm-omni, Z-Image-Turbo) and the live gate exist but have not run. |
| `dummy-model-server -strict-media` rejects `data:` images that are not base64 or smaller than 2x2, and audio uploads that are not parseable WAV/MP3 (or a known container) | `dummy-model-server/README.md`, `docs/LIMITATIONS.md` | Verified-in-code | `dummy-model-server/internal/media` tests, including the exact bytes of the former `test-data/dummy.mp3`. It is a header-level check, not a decoder. |
| The checked-in smoke matrix reports NVIDIA-only campaign measurements made with `1.0.0-rc.1` | `docs/SMOKE_RESULTS.md` | Measured | Campaign `matrix-20260915-195537` on 2026-09-15 with package `1.0.0-rc.1`; systems under test listed in that document |
| Documented limitations describe client-side measurement boundaries and output-schema behavior | `docs/LIMITATIONS.md` and `docs/OUTPUT_SCHEMA.md` | Verified-in-code | Harness behavior, tests, and the published output schema |
| `metrum-ai/prompt-library` is published on Hugging Face as a public Apache-2.0 dataset for LLM workload mixes | `docs/datasets/DATASET_CARD.md`, Hub card | Third-party public page | Hub API 2026-09-22: `private=false`, `gated=false`, `cardData.license=apache-2.0`, root `LICENSE` present; publisher asserts Apache-2.0 while upstream collection authors remain undocumented on the card |

## Maintenance

- Add a row when a new material public claim is introduced in this repository.
- Update evidence when a claim is re-measured or re-verified.
- Code existence is not live verification. A modality moves to
  Verified-live only with a campaign ID (or a named review), a date, and a
  kept data log from a real serving stack.
- Do not treat Roadmap or Modelled rows as Measured.
- Keep claims outside the repository in their channel-specific ledgers.
