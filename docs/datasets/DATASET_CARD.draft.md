<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

---
# DRAFT - not uploaded
pretty_name: "Metrum AI Bench prompt / media fixtures (draft)"
dataset_name: metrum-ai/bench-prompts
license: "PLACEHOLDER - decision pending; options CC-BY-4.0 / ODC-BY-1.0"
task_categories:
  - other
language:
  - en
tags:
  - inference-benchmarking
  - openai-compatible
  - llm
  - vlm
  - asr
size_categories:
  - n<1K
---

# Dataset Card for `metrum-ai/bench-prompts` - DRAFT - not uploaded

> This file is a **draft** Hugging Face dataset card. It has **not** been
> uploaded. License choice is pending. Do not treat filenames or splits below
> as final.

## Dataset Description

- **Repository:** https://github.com/metrum-ai/bench-cli
- **Methodology:** [docs/METRICS.md](../METRICS.md), [docs/REPRODUCING.md](../REPRODUCING.md)
- **Point of contact:** `TODO(launch): maintainer contact`

Prompt and media corpora for driving Metrum AI Bench modality runners
(LLM / VLM / ASR / imagegen) against OpenAI-compatible endpoints. This
repository's git tree ships only tiny fixtures under `test-data/`.

For **LLM workload-length mixes** (ISL/OSL targets), use the published
Apache-2.0 dataset
[`metrum-ai/prompt-library`](https://huggingface.co/datasets/metrum-ai/prompt-library)
with `metrum-ai-bench-prompts` ([PROMPT_LIBRARY.md](../PROMPT_LIBRARY.md)).
This draft card remains for other modality fixtures that may be published
separately; it does **not** claim this git tree hosts the prompt-library corpus.

## License

**Placeholder - decision pending.** Candidate options under discussion:

- CC-BY-4.0
- ODC-BY-1.0

Do not redistribute assumed terms until the chosen SPDX identifier is filled
in here and on the Hub card.

## Provenance

| Split / file (placeholder) | Source | Transformation | Notes |
|----------------------------|--------|----------------|-------|
| `llm/*.jsonl` | `TODO` | `TODO` | One JSON object per line with `prompt` |
| `vlm/*.jsonl` | `TODO` | `TODO` | `prompt` + `image_url(s)` |
| `asr/*.jsonl` | `TODO` | `TODO` | `id`, `path`/`url`, optional `duration` |
| `asr/ground_truth.jsonl` | `TODO` | `TODO` | Matching `id` + `transcript` |
| `imagegen/*.jsonl` | `TODO` | `TODO` | `prompt` / optional negatives |

## Intended use

- Load generation and performance measurement with Metrum AI Bench
- Reproducing published smoke or campaign cells when the campaign points at
  a pinned dataset revision

## Limitations

- Not a quality leaderboard corpus (except where ASR WER/CER ground truth is
  supplied)
- Image and audio URLs may rot; prefer pinned blobs in the dataset revision
- Language coverage is English-first unless stated otherwise per split

## Contamination status

`TODO(launch): document overlap checks against common pretraining / SFT corpora.`

## Training-use statement

We ask that this set **not** be used as training data. We acknowledge that we
**cannot enforce** that request.

## Versioning and immutability

Each Hub revision should be immutable. Campaigns and published results should
cite the dataset revision SHA / tag. Breaking content changes require a new
revision and a CHANGELOG note on the Hub card.

## Citation

```bibtex
@misc{metrum_ai_bench_prompts_draft,
  title        = {Metrum AI Bench Prompts (draft)},
  author       = {Metrum AI, Inc.},
  year         = {2026},
  howpublished = {\url{https://github.com/metrum-ai/bench-cli}},
  note         = {DRAFT - dataset not uploaded; license pending}
}
```

## Dataset Structure

`TODO(launch): list columns, example rows, and byte sizes after the first upload.`
