<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Superseded draft

This path used to hold a **draft** Hugging Face card for a never-uploaded
`metrum-ai/bench-prompts` dataset (mixed LLM / VLM / ASR / imagegen fixtures,
license pending).

That draft is obsolete. The LLM workload corpus **is published**:

- Hub: [huggingface.co/datasets/metrum-ai/prompt-library](https://huggingface.co/datasets/metrum-ai/prompt-library)
- Local card: [DATASET_CARD.md](DATASET_CARD.md)
- CLI: [docs/PROMPT_LIBRARY.md](../PROMPT_LIBRARY.md)

VLM, ASR, and image-generation still use tiny in-tree fixtures under
`test-data/`. They are not a Hub dataset.
