<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

---
pretty_name: "Metrum AI Prompt Library"
dataset_name: metrum-ai/prompt-library
license: Apache-2.0
task_categories:
  - text-generation
language:
  - en
tags:
  - benchmarking
  - inference
  - load-testing
  - prompts
size_categories:
  - 100K<n<1M
---

# Dataset Card for `metrum-ai/prompt-library`

This is the LLM workload-length corpus used by `metrum-ai-bench-prompts`.
It is **published** on Hugging Face:

- **Hub:** [huggingface.co/datasets/metrum-ai/prompt-library](https://huggingface.co/datasets/metrum-ai/prompt-library)
- **License:** Apache-2.0
- **Repository (CLI):** https://github.com/metrum-ai/bench-cli
- **CLI usage:** [docs/PROMPT_LIBRARY.md](../PROMPT_LIBRARY.md)
- **Methodology:** [docs/METRICS.md](../METRICS.md), [docs/REPRODUCING.md](../REPRODUCING.md)

The Hub dataset card is the canonical description of files, configs, checksums,
and rebuild scripts. This page is a bench-cli pointer so operators do not treat
the corpus as unpublished.

This git tree does **not** vendor the 593,730-row corpus. It ships only tiny
fixtures under `test-data/` ([test-data/README.md](../../test-data/README.md)).
VLM, ASR, and image-generation runners still use those local fixtures; there is
no separate Hub dataset for those modalities.

## Dataset Description

A prompt library for LLM inference workload and performance measurement with
Metrum AI Bench. Rows include prompt text, intended lengths, token buckets, and
reasoning labels. There are **no reference answers**.

| Item | Value |
|------|-------|
| Configs | `full` (default, 593,730 rows) and `sample` (2,960 rows) |
| Split | `train` (loading convention only; not a training recommendation) |
| Pin | Pass a 40-character commit SHA via `--revision` |

Pinned revision used in README examples:
`0666f62e581b482838ae2e17b333ee36ff3d01b0`.

The `full` config keeps every source record, including repeated prompt text
with distinct `target_output_length` values. Those variants are intentional:
`metrum-ai-bench-prompts` appends a word-count hint when it writes JSONL for
`metrum-ai-bench-llm`. The hint guides generation; it does not guarantee an
exact response length.

The `sample` config takes ten rows from each of 296 observed
input-bucket / output-bucket / reasoning groups. It is a balanced smoke sample,
not a frequency-representative draw of `full`.

## License

Apache-2.0, matching the Hub card and this repository.

## Fields (selection)

| Field | Role in bench-cli |
|-------|-------------------|
| `prompt` | Base prompt text (not rewritten on the Hub) |
| `target_output_length` | Intended output **word** count; appended as a generation hint |
| `target_input_tokens` | Supplied ISL token target |
| `target_output_tokens` | Supplied OSL token target / recommended per-row budget |
| `reasoning` | Metadata filter (`--reasoning any\|true\|false`) |

Legacy `prompt_length` and `actual_words` are not authoritative. Word ISL is
`len(rendered_prompt.split())` with Unicode whitespace. See
[PROMPT_LIBRARY.md](../PROMPT_LIBRARY.md) for counting scope.

## Intended use

- Selecting an ISL/OSL mix with `metrum-ai-bench-prompts`
- Load generation and performance measurement with `metrum-ai-bench-llm`
- Reproducing published cells when the campaign cites a pinned dataset revision

## Limitations

- Not a quality leaderboard corpus (no gold answers)
- Reasoning labels and requested lengths are source-supplied, not independently
  validated
- Token targets are not measurements from a named tokenizer
- Language coverage is English-first unless a future Hub revision states
  otherwise

## Training-use statement

We ask that this set **not** be used as training data. We acknowledge that we
**cannot enforce** that request. The Hub `train` split name is a loader
convention only.

## Versioning and immutability

Each Hub revision should be treated as immutable. Campaigns and published
results should cite the dataset revision SHA. Breaking content changes require
a new revision on the Hub.

## Citation

```bibtex
@misc{metrum_ai_prompt_library,
  title        = {Metrum AI Prompt Library},
  author       = {Metrum AI, Inc.},
  year         = {2026},
  howpublished = {\url{https://huggingface.co/datasets/metrum-ai/prompt-library}},
  note         = {Apache-2.0; pin a revision SHA for reproducible runs}
}
```
