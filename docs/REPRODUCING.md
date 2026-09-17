<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Reproducing the checked-in reference

Requirements: Rust 1.85+, Go 1.26.6+, and an otherwise idle local machine.

```bash
cargo build --release
(cd dummy-model-server && go run ./cmd/dummy-model-server \
  -port 18321 -latency 100ms -chunk-interval 20ms) &
```

In a second shell:

```bash
printf '%s\n' '{"prompt":"Hi"}' > /tmp/metrum-prompts.jsonl
target/release/metrum-ai-bench-llm \
  --url http://127.0.0.1:18321/v1/chat/completions --api-key dummy \
  --scenario reference --num-requests 16 --concurrency 4 \
  --prompts /tmp/metrum-prompts.jsonl --mode chat --streaming \
  --model dummy --max-tokens 20 --seed 7 --warmup-requests 0 \
  --data-log /tmp/metrum-reference.jsonl --log-level error
```

Compare the shared summary to `test-data/reference-result.json`. Scheduling
and counts must match exactly. On an unloaded machine mean TTFT should be
0.100–0.200 s, mean end-to-end latency 0.420–0.650 s, and mean ITL
0.010–0.040 s. Wall-clock timestamps, UUIDs, hostname, and exact timings are
expected to differ.

The command writes request records before the summary, so every aggregate can
be independently recalculated. Record the server revision, model revision,
tokenizer, host power state, and whether prefix caching is enabled when
publishing non-dummy results.
