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
| Configs | `sample` (smoke; default in docs) or `full` |
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

Supplied token ISL does **not** include the hint text; word ISL does. Report
`isl.counting_scope` records this.

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

```bash
metrum-ai-bench-cli-prompts \
  --config sample \
  --count 64 --seed 42 \
  --profile chat-medium \
  --output /tmp/mix.jsonl --report /tmp/mix-report.json
```

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
