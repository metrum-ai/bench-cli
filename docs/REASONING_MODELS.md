<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Reasoning models

Operator notes for benchmarking thinking / reasoning models with Metrum AI
Bench. Read this before choosing `--max-tokens` or `reasoning_effort`.

## Why this page exists

Thinking models emit reasoning deltas before the visible answer. A naive TTFT
that counts the first reasoning token makes the model look fast. A small
`--max-tokens` often never reaches the answer at all.

## How Bench measures it

Definitions (from [METRICS.md](METRICS.md)):

- **TTFT** (`ttft_s` on each request; summary distribution `ttft_s`): first
  **visible** output delta minus send. Role and reasoning-only deltas do not
  count. Missing visible output is recorded as error kind `no_output_token`
  and appears in summary `errors_by_type.no_output_token`.
- **First reasoning** (`first_reasoning_s` on each request): first non-empty
  `reasoning_content` / `reasoning` delta minus send, reported separately from
  TTFT.

There is **no** summary-level distribution for `first_reasoning_s` today.
Compare per-request values in the JSONL when you need that latency. See
[OUTPUT_SCHEMA.md](OUTPUT_SCHEMA.md).

## Passing `reasoning_effort` and related parameters

`--extra-body-json` merges a JSON object into the request body (LLM, VLM, and
imagegen). Example for OpenAI-style effort:

```bash
--extra-body-json '{"reasoning_effort":"medium"}'
```

vLLM-style thinking toggle:

```bash
--extra-body-json '{"chat_template_kwargs":{"enable_thinking":true}}'
```

The raw string is recorded on the run as `config.common.extra_body_json` and
the merged keys also appear in `config.body_template`. Set the value
explicitly on every run, including when you intend the model default, so the
manifest carries it.

`--extra-body-file` exists on `metrum-ai-bench-imagegen` (JSON object from a
file). LLM/VLM use `--extra-body-json` only. Strategic has neither flag.

## `--max-tokens` for thinking models

A cap smaller than the model's typical reasoning length at the chosen effort
produces `no_output_token` on every request and a null / empty TTFT
distribution. Do not guess a number from this page.

Probe procedure:

1. Run `--num-requests 4 --concurrency 1` with your intended
   `--extra-body-json` and a candidate `--max-tokens`.
2. Inspect the summary line: raise the cap until
   `errors_by_type.no_output_token` is absent or zero.
3. Record that cap on the manifest (it is already in `config.body_template`
   as `max_tokens`) and use it for the full sweep.

## Run-time budgeting

Higher effort means more generated tokens per request. Sweep wall time scales
with concurrency × requests × tokens. Probe at concurrency 1 first. Use
`--stop-after-seconds` on LLM/VLM/ASR as a guard on long sweeps so issuance
stops after N seconds while in-flight requests drain.

## Same model, several effort levels

One server, one model revision, one prompt set, one seed, N runs that differ
only in `reasoning_effort`, each with its own `--data-log` and the same
`--sut`:

```bash
for effort in xhigh medium low; do
  target/release/metrum-ai-bench llm -- \
    --url "$URL" --api-key "$KEY" --model "$MODEL" --mode chat --streaming \
    --scenario "effort-${effort}" \
    --prompts prompts.jsonl --num-requests 64 --concurrency 32 --seed 7 \
    --max-tokens "$MAX_TOKENS" \
    --extra-body-json "{\"reasoning_effort\":\"$effort\"}" \
    --sut sut.json --require-sut \
    --data-log "results-${effort}.jsonl"
done
```

Fields to compare across the three summaries: time-to-first-answer
(`ttft_s`), output tokens per second (`completion_tokens_per_second`), and
`errors_by_type.no_output_token`. First-reasoning latency is per-request
(`first_reasoning_s`) only; the summary has no first-reasoning distribution.
There is no reasoning-tokens-per-request field in the schema today.

## What Bench does not measure

Answer quality. See [LIMITATIONS.md](LIMITATIONS.md) "Performance, not
quality".
