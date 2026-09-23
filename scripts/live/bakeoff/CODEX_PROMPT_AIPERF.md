<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Codex prompt: AIPerf half of the LLM bake-off

You are on a Shadeform GPU host. Run the [ai-dynamo/aiperf](https://github.com/ai-dynamo/aiperf)
half of an LLM loadgen bake-off against the same local endpoint and the same
prompt-library mix used for Metrum.

Constraints:
- Stay on this host. Loopback only.
- Time every phase (Python/venv install is expected to be slower than a binary
  install; record actuals).
- Match model, max tokens / OSL policy, concurrency ladder, and warmup policy
  as closely as AIPerf allows; document any mismatches.
- Write artifacts next to the Metrum outputs under
  `docs/reviews/bakeoff/<date>-<sku>/`.
- Do not print API secrets.

Install AIPerf in a venv, convert or feed the same JSONL mix, run the sweep,
and summarize peak tok/s, latency percentiles, saturation stability, and phase
timings for the comparison report.
