<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Reproducing the checked-in reference

Requirements: Rust 1.89+, Go 1.26.10+, and an otherwise idle local machine.

```bash
cargo build --release
(cd dummy-model-server && go run ./cmd/dummy-model-server \
  -port 18321 -latency 100ms -chunk-interval 20ms) &
```

In a second shell:

```bash
printf '%s\n' '{"prompt":"Hi"}' > /tmp/metrum-prompts.jsonl
target/release/metrum-ai-bench-cli-llm \
  --url http://127.0.0.1:18321/v1/chat/completions --api-key dummy \
  --scenario reference --num-requests 16 --concurrency 4 \
  --prompts /tmp/metrum-prompts.jsonl --mode chat --streaming \
  --model dummy --max-tokens 20 --seed 7 --warmup-requests 0 \
  --data-log /tmp/metrum-reference.jsonl --log-level error
```

## Prompt-sequence and issuance comparability (#313)

An equal `--seed` is NOT proof of matched workload: the same seed and
canonical pool can yield different effective prompt orders across clients,
and closed-loop replacement waves can phase-lock (synchronized in one run,
dephased in another). The llm binary ships the comparability knobs from the
W2 TTFT audit:

- `--prompt-sequence-log PATH` writes the **effective** canonical sequence
  per slot (`seq`, `prompt_index`, `prompt_sha256_16`, `prompt_bytes`,
  `cohort`) — fingerprints and lengths only, never prompt text. Compare
  logs to validate workload equivalence before comparing TTFT.
- `--prompt-sequence PATH` replays a recorded sequence exactly: entries
  resolve by content fingerprint (a prescription from another client
  transfers even when the shuffle mapping differs), the seeded shuffle is
  skipped, and content missing from the pool fails before traffic — never
  silently substituted.
- `--start-jitter-ms MS [--start-jitter-seed S]` adds a seeded uniform
  per-slot start stagger for fixed-concurrency runs, breaking persistent
  phase-locking of replacement waves. Opt-in only; recorded in the run
  config; never applied to scored runs by default.
- `--sse-trace-first-n N` retains a bounded, redacted SSE event timeline
  (event index, ms-from-send, categories) for the first N requests in
  `<data-log>.sse-trace.jsonl` — no prompt text, content, or secrets —
  for auditing first-visible vs first-reasoning timing.
- Summaries report the two clocks separately: `ttft_s` (first visible
  content token) and, when a reasoning row exists, `ttft_first_any_s`
  (first token of any category).

Compare the shared summary to `test-data/reference-result.json`. Scheduling
and counts must match exactly. On an unloaded machine mean TTFT should be
0.100–0.200 s, mean end-to-end latency 0.420–0.650 s, and mean ITL
0.010–0.040 s. Wall-clock timestamps, UUIDs, hostname, and exact timings are
expected to differ.

The command writes request records before the summary, so every aggregate can
be independently recalculated. Record the server revision, model revision,
tokenizer, host power state, and whether prefix caching is enabled when
publishing non-dummy results.
