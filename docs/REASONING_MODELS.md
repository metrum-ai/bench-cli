<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Reasoning models

**Search first.** Thinking toggles, reasoning parsers, and effort levels
differ by model and engine version. Before a real run, check the model card
and the engine's reasoning docs (for vLLM:
https://docs.vllm.ai/en/latest/features/reasoning_outputs.html). Then set the
request fields explicitly with `--extra-body-json` so the manifest records
them.

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
  TTFT. The summary carries a `first_reasoning_s` distribution over measured
  successes that streamed a reasoning delta.
- **Reasoning tokens** (`reasoning_tokens` and `visible_completion_tokens` on
  each request; summary distributions plus `reasoning_tokens_total` and
  `visible_completion_tokens_total`): the server-reported reasoning count
  from `usage`, and `completion_tokens` minus that count. `null` (never `0`)
  when the server does not report it; Bench does not estimate it with a
  tokenizer. vLLM (the default engine, [SERVING.md](SERVING.md)) reports
  `usage.completion_tokens_details.reasoning_tokens` from v0.28.0
  ([vllm#45802](https://github.com/vllm-project/vllm/pull/45802)), and only
  when started with `--reasoning-parser`
  ([vLLM reasoning outputs](https://docs.vllm.ai/en/latest/features/reasoning_outputs/)).
  Without the parser, or on older vLLM, the field is `null`, so you will see
  `reasoning_tokens.n=0` even though the model reasons. Add the model's
  reasoning parser to the launch flags if you need the count. Other engines:
  check their docs for `completion_tokens_details`. See
  [METRICS.md](METRICS.md) for the accepted locations.

See [OUTPUT_SCHEMA.md](OUTPUT_SCHEMA.md).

### Reading `reasoning_tokens` vs `no_output_token`

The two answer different questions:

- `no_output_token` is a request failure: the stream ended with no visible
  answer delta, usually because `--max-tokens` ran out during reasoning.
  These rows are excluded from summary distributions and their
  `reasoning_tokens` is `null` (the request errored), so they never show up
  in the reasoning token summary.
- `reasoning_tokens` describes successful requests: how much of each
  answer's `completion_tokens` went to thinking.

So a run with many `no_output_token` errors and a modest
`reasoning_tokens` p50 means the cap cut off the long tail, and the
distribution is biased low. Fix the cap first (see below). Once
`no_output_token` is zero, compare `reasoning_tokens` p99 with
`--max-tokens` to see the remaining headroom. `completion_tokens`,
`completion_tokens_per_second`, and cost per million output tokens include
reasoning tokens; use `visible_completion_tokens` for answer-only counts.

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

`--extra-body-file` exists on `metrum-ai-bench-cli-imagegen` (JSON object from a
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
  target/release/metrum-ai-bench-cli llm -- \
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
`errors_by_type.no_output_token`, plus first-reasoning latency
(`first_reasoning_s`), `reasoning_tokens` / `reasoning_tokens_total`, and
`visible_completion_tokens` when the server reports reasoning usage.

## What Bench does not measure

Answer quality. See [LIMITATIONS.md](LIMITATIONS.md) "Performance, not
quality".
