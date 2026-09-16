<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# JSONL output schema

Each line is a complete JSON object and carries `schema_version`.

## Request v3

`metrum-ai-bench.request.v3` records are flushed immediately on completion:

- optional `run_id` (same UUID as `summary.config.run_id` when stamped)
- `seq`, `phase` (`warmup`, `measure`, `drain`), and `endpoint`
- ISO `started_at`/`completed_at`
- optional monotonic `send_offset_s` (seconds from the run-epoch `Instant` to
  actual send; preferred for window and closed-loop bins)
- monotonic `latency_s`, optional `ttft_s`, `first_byte_s`, `first_reasoning_s`, and `itl_s`
  (`ttft_s` is null for non-streaming LLM/VLM responses; it is never fabricated
  from E2E latency)
- optional `scheduled_offset_s` and `queue_delay_s`
- server usage counts plus optional `tokenized_*` counts and `usage_missing`
- typed `error`, `partial`, modality-specific numeric metrics, and optional
  string `modality_labels` (e.g. imagegen `artifact_0_sha256`)

`modality_metrics` holds flat numeric values keyed by modality:

| Binary | Keys |
|--------|------|
| VLM | `image_count`, `image_bytes` (bytes actually sent per request) |
| ASR | `rtfx_client`, `wer`, `cer`, `inference_seconds_{server,client}` |
| Imagegen | `images_requested`, `images_returned`, `response_bytes`, `artifact_N_bytes` |

Imagegen artifact SHA-256 digests live in `modality_labels.artifact_N_sha256`
on `request.v3` (no parallel `imagegen.request.v1` lines).

`first_byte_s` is the monotonic elapsed time from send start until response
headers are received (after a successful `.send()`). It is distinct from
`ttft_s` (first visible user token), which includes connect/TLS/queue. A
separate `connect_s` field is deferred (post-v1 / feature-flagged).

## Summary v3

`metrum-ai-bench.summary.v3` is field-additive over v2. It contains measured
attempted/success/error counts, rates, type-7 distributions, coordinated-omission-
corrected latency, throughput-bin dispersion, SLO goodput, `pooled_mixture`,
full `per_endpoint` distributions, environment metadata, and `partial`.

Additional v3 fields:

- `config` — effective run configuration:
  - `run_id` — UUID generated once per run
  - `effective_max_concurrency` — outstanding-request cap in force
    (`--max-concurrency`, or `--concurrency` when unset)
  - `common` — every `CommonBenchArgs` field (`seed`, `warmup_requests`,
    `request_rate`, `arrival`, `max_concurrency`, `load_balancer`, `ignore_eos`,
    `min_tokens`, `extra_body_json`, `system_prompt`, `unique_prompts`,
    `tokenizer`, `slos`, `throughput_bin_seconds`, `insecure`, optional
    `ca_cert` path). Secrets are never stamped.
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

Unversioned / legacy dual-summary objects are no longer written. Console output
and JSONL both derive from `RunSummary` / `DistSummary` (Hyndman–Fan type 7).
Historical `request.v2` / `summary.v2` lines from 0.1.82 remain readable for
regression audit (`record::accepts_audit_schema`); new campaign validation
rejects any JSONL line without `schema_version`.
