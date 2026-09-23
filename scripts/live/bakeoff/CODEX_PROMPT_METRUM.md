<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Codex prompt: Metrum half of the LLM bake-off

You are on a Shadeform GPU host. Run the Metrum AI Bench CLI half of an LLM
loadgen bake-off against the local OpenAI-compatible server on loopback.

Constraints:
- Stay on this host. Use loopback URLs only (no public WAN loadgen).
- Time every phase with `scripts/live/bakeoff/phase_timer.sh` into
  `timings.jsonl` (install, prompt extract, SUT, sweep stages, export).
- Use Hugging Face `metrum-ai/prompt-library` via `metrum-ai-bench-cli prompts`
  with a pinned revision and `--profile` when available.
- Fill `--sut` / `--require-sut`. Prefer vendor Docker serving (already running).
- Web-search current vendor-default serve flags for this model/engine and record
  sources in the SUT notes.
- Artifacts under `docs/reviews/bakeoff/<date>-<sku>/`.
- Do not print API secrets.

Then run `scripts/live/bakeoff/run_on_host.sh` for the metrum path (or the
equivalent commands by hand) and summarize tok/s, TTFT/TPOT, observed
concurrency, and wall-clock phase times.
