<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# JSONL output schema

Each line is a complete JSON object and carries `schema_version`.

## Request v3

`metrum-ai-bench.request.v3` records are flushed immediately on completion:

- optional `run_id` (same UUID as `summary.config.run_id` when stamped)
- `seq`, `phase` (`warmup`, `measure`, `drain`), and `endpoint`
- ISO `started_at`/`completed_at`
- monotonic `latency_s`, optional `ttft_s`, `first_reasoning_s`, and `itl_s`
- optional `scheduled_offset_s` and `queue_delay_s`
- server usage counts plus optional `tokenized_*` counts and `usage_missing`
- typed `error`, `partial`, and modality-specific numeric metrics

`modality_metrics` holds flat numeric values keyed by modality:

| Binary | Keys |
|--------|------|
| VLM | `image_count`, `image_bytes` (bytes actually sent per request) |
| ASR | `rtfx_client`, `wer`, `cer`, `inference_seconds_{server,client}` |
| Imagegen | `images_requested`, `images_returned` |

Image-generation request records retain artifact hashes and response details
under their modality schema because those fields are not token-oriented.

## Summary v3

`metrum-ai-bench.summary.v3` is field-additive over v2. It contains measured
attempted/success/error counts, rates, type-7 distributions, coordinated-omission-
corrected latency, throughput-bin dispersion, SLO goodput, `pooled_mixture`,
full `per_endpoint` distributions, environment metadata, and `partial`.

Additional v3 fields:

- `config` — effective run configuration:
  - `run_id` — UUID generated once per run
  - `common` — every `CommonBenchArgs` field (`seed`, `warmup_requests`,
    `request_rate`, `arrival`, `max_concurrency`, `load_balancer`, `ignore_eos`,
    `min_tokens`, `extra_body_json`, `system_prompt`, `unique_prompts`,
    `tokenizer`, `slos`, `throughput_bin_seconds`)
  - `effective_system_prompt` — system string actually sent (omitted/`null` when
    N/A or disabled); VLM currently records its hardcoded image-capable default
  - `body_template` — sanitized request skeleton with a `{{prompt}}` placeholder
    (no secrets, no raw images/audio)
  - `unique_prompt_nonce_template` — present when `--unique-prompts` is on:
    `[nonce-{run_id}-{seed}-{seq}]`
- `usage_missing_count` — measure-phase successes with `usage_missing`
- `completion_tokens_per_second` — `null` when any measured success has
  `usage_missing` without a tokenizer count to fill the gap; otherwise a rate
- `completion_tokens_source` — `"server_usage"` or `"tokenizer_fallback"` when
  the rate is present

Every `DistSummary` carries `p90_unreliable`, `p95_unreliable`, and
`p99_unreliable` using `percentile_unreliable(n, p)` (unreliable when
`n * (1 - p/100) < 1`).

Warmup request lines remain in the file for audit but are excluded from
summary distributions. Ctrl-C stops issuance, drains started requests, and
writes a partial summary. A hard kill may leave valid request lines without a
summary; consumers must accept that recoverable prefix.

Legacy summary objects are temporarily appended by the modality binaries for
existing consumers. New consumers must select records by `schema_version`,
not line position.
