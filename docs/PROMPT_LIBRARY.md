<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Prompt library extractor

`metrum-ai-bench-cli-prompts` selects a reproducible mix from the Hugging Face
dataset
[`metrum-ai/prompt-library`](https://huggingface.co/datasets/metrum-ai/prompt-library)
and writes JSONL that `metrum-ai-bench-cli-llm` can consume with `--prompts`.

Success is **ISL and OSL statistics within absolute CLI tolerances**, not an
exact row count. `--count` is a preferred size; the selector may return fewer
or more rows (within `--count-slack`) and may **repeat** source rows when that
is required to land both axes.

## Dataset

| Item | Value |
|------|-------|
| Repository | `metrum-ai/prompt-library` |
| Configs | `sample` (smoke; clap default) or `full` (publishable default) |
| Split | `train` |
| Default revision | `main` (latest Hub commit; resolved SHA is written to `--report`) |
| Pin (optional) | Pass a 40-character commit SHA via `--revision`, or `--require-pinned-revision` |

Other Metrum AI prompt sets published on Hugging Face under the `metrum-ai`
organization can be used the same way; record the dataset name, **resolved**
revision SHA from the mix report, and row count in the SUT block or run notes.

### Fields used for selection

| Field | Role |
|-------|------|
| `prompt` | Base prompt text (never rewritten in the dataset) |
| `target_output_length` | Intended output **words**; appended as a generation hint |
| `target_input_tokens` | Supplied ISL token target (`--isl-unit tokens --isl-token-basis supplied-target`) |
| `target_output_tokens` | Supplied OSL token target / recommended per-row budget |
| `reasoning` | Metadata filter only (`--reasoning any\|true\|false`) |

Legacy `prompt_length` / `actual_words` are **not** authoritative. Word ISL is
`len(rendered_prompt.split())` with Unicode whitespace (Python `str.split()`
semantics). The rendered prompt is:

```text
{prompt}

Please aim for approximately {target_output_length} words in your response.
```

Supplied token ISL does **not** include the hint text or the server's chat
template; word ISL does include the hint. `--isl-token-basis` currently accepts
only `supplied-target`, which reads `target_input_tokens`. Report
`isl.counting_scope` records this. Because the server tokenizer and template
define runtime ISL, calibrate the selected prompts with the server's
`/tokenize` endpoint when it provides one, then compare several real request
`usage.prompt_tokens` values. Record the tokenizer, template, and observed
offset. If the endpoint has no compatible `/tokenize`, use measured usage from
a low-concurrency calibration run. Do not describe supplied-target ISL as a
server token count.

## Named workload profiles

Prefer `--profile` for publishable compares so ISL/OSL targets stay versioned
and comparable across runs. Profiles are tokens / median unless you override
`--isl-stat` / `--osl-unit`. Explicit `--isl-target` / `--osl-target` conflict
with `--profile`. Zero CLI tolerances fall back to the profile defaults.

| Profile | Version | ISL | OSL | Default tolerances |
|---------|---------|-----|-----|--------------------|
| `chat-short` | 1 | 256 | 64 | 32 / 16 |
| `chat-medium` | 1 | 512 | 128 | 64 / 32 |
| `rag-medium` | 1 | 2048 | 256 | 128 / 64 |
| `summarize-long` | 1 | 4096 | 512 | 256 / 64 |
| `code-medium` | 1 | 1024 | 512 | 128 / 64 |

For a publishable mix, use the full config and a named profile, and pass both
tolerances explicitly so the command records the intended windows:

```bash
metrum-ai-bench-cli-prompts \
  --config full \
  --count 64 --seed 42 \
  --profile chat-medium \
  --isl-tolerance 64 --osl-tolerance 32 \
  --output /tmp/mix.jsonl --report /tmp/mix-report.json
```

Use `--config sample --profile chat-short` only for smoke tests.

The mix report includes the **resolved** Hub commit SHA under `revision`, plus
`profile.name` and `profile.version` when a profile was used.

To freeze a publishable compare, pass an explicit SHA (from a prior report or
Hub) and optionally `--require-pinned-revision`:

```bash
metrum-ai-bench-cli-prompts \
  --revision 0666f62e581b482838ae2e17b333ee36ff3d01b0 \
  --require-pinned-revision \
  --config sample \
  --count 64 --seed 42 \
  --profile chat-medium \
  --output /tmp/mix.jsonl --report /tmp/mix-report.json
```

## CLI knobs

```bash
metrum-ai-bench-cli-prompts \
  --config sample \
  --count 64 --count-slack 64 --seed 42 \
  --isl-target 512 --isl-unit tokens --isl-stat median --isl-tolerance 64 \
  --osl-target 128 --osl-unit tokens --osl-stat median --osl-tolerance 32 \
  --output /tmp/mix.jsonl --report /tmp/mix-report.json
```

| Flag | Meaning |
|------|---------|
| `--revision` | Hub ref (default `main` = latest). Pass a 40-char SHA to pin |
| `--require-pinned-revision` | Fail unless `--revision` is a 40-char SHA |
| `--allow-moving-revision` | Deprecated no-op (floating refs always resolve) |
| `--profile` | Named versioned ISL/OSL pair (see table above) |
| `--count` | Preferred mix size (soft) |
| `--count-slack` | Max \|actual − preferred\| (default `max(count, 32)`) |
| `--isl-stat` / `--osl-stat` | `mean` or `median` (even-n median = mean of two central values) |
| `--isl-tolerance` / `--osl-tolerance` | Absolute tolerances in the axis units |
| `--max-repeats` | Cap copies of one source row (default 8) |
| `--no-repeats` | Strict without-replacement (`--max-repeats 1`) |
| `--osl-tokens-per-word` | Required when `--osl-unit words`; used only to recommend `--max-tokens` |
| `--local-jsonl` / `--local-parquet` | Offline / test inputs (skip Hub) |
| `--cache-dir` / `--offline` | Hub cache control |

When several mixes all sit inside tolerance, the selector prefers solutions
closer to `--count` and with fewer repeats.

## Outputs

- **JSONL** (`--output`): one object per selected slot, including intentional
  repeats. `prompt` already includes the word-count hint. Extra fields
  (`source_ordinal`, `target_output_tokens`, …) are ignored by llm today.
- **Report** (`--report`): resolved Hub revision SHA, preferred vs `selected_count`,
  achieved ISL/OSL and gaps, repeat histogram, `recommended_max_tokens`,
  `recommended_num_requests`, schedule SHA-256, optional `profile`.

## Feeding `metrum-ai-bench-cli-llm`

`metrum-ai-bench-cli-llm` still uses a **global** `--max-tokens` and cycles a
shuffled prompt pool. To preserve the selected mix:

1. Set `--num-requests` to `report.selected_count` (not blindly to `--count`).
2. Set `--warmup-requests 0` (warmup would drop the first W measured slots).
3. Set `--max-tokens` to `report.recommended_max_tokens`.
4. Do not set `num_requests != selected_count` (cycling changes the mix).

Mean-target example (same dummy-server loop as the README median example):

```bash
target/release/metrum-ai-bench-cli-prompts \
  --config sample \
  --count 32 --seed 7 \
  --isl-target 256 --isl-unit tokens --isl-stat mean --isl-tolerance 32 \
  --osl-target 128 --osl-unit tokens --osl-stat mean --osl-tolerance 16 \
  --output /tmp/mix-mean.jsonl --report /tmp/mix-mean-report.json
```

## Complete gated sweep

This example selects a publishable prompt mix, preserves every selected row in
each strategic stage, constrains the output-token window, and enables the
runtime OSL gate. Replace the endpoint, model, SUT, and revision with values
for the system under test. Calibrate server-token ISL as described above and
gate each strategic NDJSON request row's `input_tokens` in the campaign runner;
the CLI counts ISL mismatches but has no `--fail-on-isl-mismatch` flag.

```bash
set -euo pipefail
REPORT=/tmp/chat-medium-report.json
PROMPTS=/tmp/chat-medium.jsonl
metrum-ai-bench-cli-prompts \
  --config full --revision REVISION_SHA --require-pinned-revision \
  --count 128 --seed 7 --profile chat-medium \
  --isl-tolerance 64 --osl-tolerance 32 \
  --output "$PROMPTS" --report "$REPORT"
selected_count="$(jq -r .selected_count "$REPORT")"
max_tokens="$(jq -r .recommended_max_tokens "$REPORT")"
min_tokens=$((max_tokens > 32 ? max_tokens - 32 : 1))
metrum-ai-bench-cli-strategic \
  --url https://SERVER/v1/chat/completions --api-key "$API_KEY" \
  --model MODEL --streaming --prompts "$PROMPTS" \
  --prompt-mix-report "$REPORT" \
  --isl-tolerance 64 --osl-tolerance 32 --fail-on-osl-mismatch \
  --min-tokens "$min_tokens" --max-tokens "$max_tokens" --ignore-eos \
  --warmup-requests 0 --requests-per-stage "$selected_count" \
  --sweep 1,2,4,8,16 --sut sut.json --require-sut \
  --ndjson run.ndjson --csv requests.csv --html report.html
```

`--fail-on-osl-mismatch` checks successful requests in the last strategic
stage and exits nonzero after writing the summary, CSV, and HTML if any
measured output differs from its target by more than the tolerance. It does
not gate ISL.

The following small tagged block is the CI fixture for the documentation
command checker. It uses the mock server that the checker starts.

<!-- doc-check -->
```bash
curl --fail --silent "$MOCK_URL/v1/models" | grep -q 'metrum-ai-bench-cli-mock'
```

## Failures

If no mix in the allowed size band hits both tolerances, the tool exits
nonzero **before** writing `--output` / `--report`. Diagnostics include best
ISL/OSL gaps, preferred vs best `n`, candidate count, work used, and whether
the failure is a proven empty candidate set vs search/tolerance miss.

## See also

- [LIMITATIONS.md](LIMITATIONS.md): mix fidelity and global token-cap caveats
- [CLI.md](CLI.md): regenerated `--help` text
- Dataset card: [datasets/DATASET_CARD.md](datasets/DATASET_CARD.md)
- Hub: [metrum-ai/prompt-library](https://huggingface.co/datasets/metrum-ai/prompt-library)
