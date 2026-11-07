<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Reasoning models

For an exact model with a recorded reasoning configuration, use that entry
and its sources. If the model is not exact or has no entry, check the model
card and engine reasoning docs (for vLLM:
https://docs.vllm.ai/en/latest/features/reasoning_outputs.html). Set request
fields explicitly with `--extra-body-json` so the manifest records them.

Operator notes for benchmarking thinking / reasoning models with Metrum AI
Bench. Read this before choosing `--max-tokens` or `reasoning_effort`.

## Why this page exists

Thinking models emit reasoning deltas before the visible answer. First-any
TTFT measures generation onset, not answer availability. A small `--max-tokens`
often never reaches the answer at all.

## How Bench measures it

Definitions (from [METRICS.md](METRICS.md)):

- **TTFT** (`ttft_s`): first nonempty reasoning or visible output delta minus
  send. `ttft_visible_answer_s` measures the first visible output separately.
  Role-only and HTTP header events are not tokens. A reasoning-only stream with
  `finish_reason: length` is a `reasoning_truncated` outcome, non-failing by
  default; other no-answer streams are `no_output_token` errors. Use
  `--strict-reasoning-truncation` or `--ci-safety` to classify truncation as an
  error. Metrum AI v1.7.2 used visible TTFT as its headline: rebaseline SLOs.
- **First reasoning** (`first_reasoning_s` on each request): first non-empty
  `reasoning_content` / `reasoning` delta minus send, reported separately from
  TTFT. `first_reasoning_s` and `ttft_first_any_s` summarize measured streams
  with observed deltas, including completed no-answer streams.
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

- `reasoning_truncated` is a counted no-answer outcome where the token limit
  ended reasoning; strict mode turns it into a request failure. Other no-answer
  streams are `no_output_token`. Observed clocks and usage are retained.
- `reasoning_tokens` describes successful requests only. The separate
  `observed_reasoning_tokens` and `observed_reasoning_tokens_total` also
  include reporting no-answer streams; absent usage remains null.

Inspect truncation and answer coverage before using results. Compare observed
reasoning-token tails with `--max-tokens` to estimate headroom. Completion
throughput includes reported reasoning across the full measured window and is
null when usage is incomplete. `visible_completion_tokens` is the reported
completion-minus-reasoning split, not necessarily an exact answer-token count.

## Passing `reasoning_effort` and related parameters

A `null` value in `--extra-body-json` removes that key from the request body.
For models requiring the newer limit field, use
`--extra-body-json '{"max_tokens":null,"max_completion_tokens":256}'` together
with `--max-tokens 256`. Set `--osl-target 256` when validating output
length; the override does not change the OSL target. This is a top-level
merge; explicit top-level nulls cannot be sent.

`--extra-body-json` merges a JSON object into the request body (LLM, VLM,
strategic chat/VLM, and imagegen). Example for OpenAI-style effort:

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
file). LLM/VLM and strategic chat/VLM use `--extra-body-json`.

## `--max-tokens` for thinking models

A cap smaller than the model's typical reasoning length at the chosen effort
can produce `reasoning_truncated` on every request and a null / empty visible
TTFT distribution. Do not guess a number from this page.

Probe procedure:

1. Run `--num-requests 4 --concurrency 1` with your intended
   `--extra-body-json` and a candidate `--max-tokens`.
2. Inspect truncation and answer coverage; if answers are required, raise the
   cap within the model and server context limits until coverage is adequate.
3. Record that cap on the manifest (it is already in `config.body_template`
   as `max_tokens`) and use it for the full sweep.

## Clock and fallback reference

All clocks below are elapsed seconds from request send, not wall-clock timestamps.
See the request and summary schemas in [OUTPUT_SCHEMA.md](OUTPUT_SCHEMA.md).

| Field | Meaning |
| --- | --- |
| `first_reasoning_s` | First nonempty reasoning delta; null if none was observed. |
| `ttft_first_any_s` | Earliest observed reasoning or visible delta; an observed stream clock, not an HTTP approximation. |
| `ttft_s` | Headline resolved first-any TTFT; may use the explicitly enabled HTTP approximation when no token clock is available. Check `ttft_source`. |
| `ttft_visible_answer_s` | First visible answer delta, independently of reasoning; null if no visible answer was observed. |

`no_output_token` means a completed stream did not provide an answer and did
not qualify for the reasoning-only length-truncation outcome. Reasoning can
therefore be observed even when this error is present. Increasing the output
cap is not a universal remedy: inspect the finish reason and server response.

- `--allow-missing-ttft` permits missing TTFT coverage in the chat audit. It does
  not invent a time, turn an unanswered response into an answer, or erase
  `no_output_token` errors. Error/strict-mode exit checks still apply.
- `--ttft-fallback first-token` is a deprecated, redundant compatibility flag:
  actual reasoning tokens already participate in first-any TTFT by default.
- `--infer-ttft-from-first-byte` only supplies a missing headline clock from
  HTTP response-header arrival. It labels the value `first_byte_approx` in
  `ttft_source` and reports approximation coverage. Headers are not output
  tokens, and the value must not be described as visible-answer timing.
  It does not replace an observed streaming token time. The CLI makes it
  mutually exclusive with `--ttft-fallback`.

## Unattended harnesses running thinking models

Report headline `ttft_s`, `ttft_visible_answer_s`, `first_reasoning_s`,
`ttft_coverage`, answer coverage and outcomes. First-token time is not answer
latency. `--infer-ttft-from-first-byte` uses HTTP headers and is explicitly
labelled, not a token time. `--ttft-fallback first-token` is redundant under
first-any TTFT; `--allow-missing-ttft` relaxes only the missing-time audit.
`--strict-reasoning-truncation` classifies no-answer length truncation as an
error and exits nonzero on a measured truncation; `--ci-safety` enforces
strictness and fails on errors. Inspect
coverage rather than relying on exit status. Missing token usage yields null
throughput; the non-reasoning split requires server-reported reasoning usage.

When answers are required, increase `--max-tokens` and `--request-timeout`
(strategic: `--timeout-seconds`) enough for the workload. Alternatively, turn
thinking off only when the model/server supports it and the different behavior
is intended:

```bash
--extra-body-json '{"chat_template_kwargs":{"enable_thinking":false}}'
```

The streaming measurements use choice index 0. Separate models and thinking
settings in comparisons; never interpret a fallback as visible-answer latency.

## Preflight on thinking models

`metrum-ai-bench-cli preflight` checks streaming with one request capped at
`max_tokens` 8. A thinking model (for example Qwen3 served by vLLM with
`--reasoning-parser qwen3`) can spend all 8 tokens on reasoning, so no
visible content arrives. A streamed reasoning delta (`delta.reasoning_content`
or `delta.reasoning`) still proves that streaming works, so
`streaming_first_token` reports PASS with
`first token was reasoning in N ms (no visible content within the probe's max_tokens)`
(#230). The check still fails when the stream ends with neither content nor
reasoning.

To make the probe return visible content, turn thinking off for preflight
only:

```bash
metrum-ai-bench-cli preflight --url http://127.0.0.1:8000 --api-key dummy \
  --model Qwen/Qwen3-8B \
  --extra-body-json '{"chat_template_kwargs":{"enable_thinking":false}}'
```

This does not change the benchmark run. Pick the thinking setting and
`--max-tokens` for the run itself as described above.

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
(`ttft_visible_answer_s`), output tokens per second (`completion_tokens_per_second`), and
truncation/answer coverage and `errors_by_type.no_output_token`, plus first-reasoning latency
(`first_reasoning_s`), `reasoning_tokens` / `reasoning_tokens_total`, and
`visible_completion_tokens` when the server reports reasoning usage.

## What Bench does not measure

Answer quality. See [LIMITATIONS.md](LIMITATIONS.md) "Performance, not
quality".
