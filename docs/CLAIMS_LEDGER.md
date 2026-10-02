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
| LLM measurement works against a real serving stack | `README.md` | Verified-live (`widen-shadeform-20261002`, 2026-10-02; pending CEO ledger sign-off) | Shadeform Scaleway 1x H100 PCIe; `Qwen/Qwen3.8-27B-FP8` on `vllm/vllm-openai:v0.30.0`. G5/MB 200/200 at ~54.5 output tok/s; strategic c=1/4/16 PASS. Artifacts: `artifacts/live/widen-20261002T144347Z/`; confirmed again in `publish-20261002T162512Z` (G5/MB). Earlier 2026-10-01 readiness review retained as prior evidence. |
| VLM measurement works against a real serving stack | `README.md` | Verified-live (`widen-shadeform-20261002`, 2026-10-02; pending CEO ledger sign-off) | Shadeform massedcompute 1x H100 PCIe; `Qwen/Qwen3-VL-32B-Instruct-FP8` on `vllm/vllm-openai:v0.30.0`. Cells c8/c16/c32 all 64/64 with inline image parts (`assert_headline.sh vlm` PASS). Artifacts: `artifacts/live/widen-parallel-vlm-20261002T153627Z/`; confirmed in `publish-20261002T162512Z` (c8/c16/c32). |
| ASR measurement works against a real serving stack | `README.md`, `docs/ASR.md` | Verified-live (`widen-shadeform-20261002`, 2026-10-02; pending CEO ledger sign-off) | Shadeform massedcompute 1x H100 PCIe; `openai/whisper-large-v3-turbo` on plain vLLM 0.30.0 (`--max-model-len 448`). Cells c1 60/60 and c8 960/960; mean WER/CER 0.0000 on `test-data/asr/` ground truth (independent verbose_json probe also WER 0). Artifacts: `artifacts/live/widen-parallel-asr-20261002T153555Z/`; confirmed in `publish-20261002T162512Z` (WER 0). vLLM-Omni ASR still blocked by [vllm-omni#5722](https://github.com/vllm-project/vllm-omni/issues/5722). |
| Image-generation measurement works against a real serving stack | `README.md` | Verified-live (`widen-shadeform-20261002`, 2026-10-02; pending CEO ledger sign-off) | Shadeform massedcompute 1x H100 PCIe; `Tongyi-MAI/Z-Image-Turbo` on `vllm/vllm-omni:v0.30.0 --omni`. Gate cell c2 4/4 and c1 8/8 with decoded artifacts; smoke-scale only (not a throughput claim). As-run seed increment bug under `--prompt` fixed in findings; re-smoke optional. Artifacts: `artifacts/live/widen-parallel-imagegen-20261002T153737Z/`; confirmed in `publish-20261002T162512Z` on L40S with unique increment seeds. |
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
