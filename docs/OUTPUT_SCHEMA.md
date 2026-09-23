<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# JSONL output schema

Each line is a complete JSON object and carries `schema_version`.

## Request v3

`metrum-ai-bench-cli.request.v3` records are flushed immediately on completion:

- optional `run_id` (same UUID as `summary.config.run_id` when stamped)
- `seq`, `phase` (`warmup`, `measure`, `drain`), and `endpoint`
- ISO `started_at`/`completed_at`
- optional monotonic `send_offset_s` (seconds from the run-epoch `Instant` to
  actual send; preferred for window and closed-loop bins)
- monotonic `latency_s`, optional `ttft_s`, `first_byte_s`, `connect_s`,
  `prefill_s`, `decode_s`, `decode_tok_s`, `first_reasoning_s`, and `itl_s`
  (`ttft_s` is null for non-streaming LLM/VLM responses; it is never fabricated
  from E2E latency)
- optional `in_flight_at_send` (client outstanding count at send)
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
`ttft_s` (first visible user token), which includes connect/TLS/queue.

`connect_s` is the HTTP connector duration for establishing a new TCP/TLS
session (DNS + TCP + TLS). `0.0` means a pooled connection was reused. Prefill
and decode proxies: `prefill_s ≈ ttft_s - connect_s` (or `ttft_s` when connect
is absent); `decode_s = max(0, latency_s - ttft_s)`;
`decode_tok_s = completion_tokens / decode_s`.

## Summary v3

`metrum-ai-bench-cli.summary.v3` is field-additive over v2. It contains measured
attempted/success/error counts, rates, type-7 distributions, coordinated-omission-
corrected latency, throughput-bin dispersion, SLO goodput, `pooled_mixture`,
full `per_endpoint` distributions, environment metadata, and `partial`.

Additional v3 fields:

- `sut` - optional operator-declared system-under-test block (rc.5). Always present in JSON; `null` when `--sut` was not provided. Fields are labelled `provenance: "declared"` (not observed). See `--sut`, `--require-sut`, and `--redact-hostname`.
- `environment.hostname` - may be `null` when `--redact-hostname` (or `--require-sut`, which implies redaction) is set (rc.5). Readers must treat `sut` and `hostname` as optional.
- `config` - effective run configuration:
  - `run_id` - UUID generated once per run
  - `effective_max_concurrency` - outstanding-request cap in force
    (`--max-concurrency`, or `--concurrency` when unset)
  - `common` - every `CommonBenchArgs` field (`seed`, `warmup_requests`,
    `request_rate`, `arrival`, `max_concurrency`, `load_balancer`, `ignore_eos`,
    `min_tokens`, `extra_body_json`, `system_prompt`, `unique_prompts`,
    `tokenizer`, `slos`, `throughput_bin_seconds`, `insecure`, optional
    `ca_cert` path) plus optional `scenario` (the modality `--scenario` label).
    Secrets are never stamped.
  - `effective_system_prompt` - system string actually sent (omitted/`null` when
    N/A or disabled); VLM currently records its hardcoded image-capable default
  - `body_template` - sanitized request skeleton with a `{{prompt}}` placeholder
    (no secrets, no raw images/audio)
  - `unique_prompt_nonce_template` - present when `--unique-prompts` is on:
    `[nonce-{run_id}-{seed}-{seq}]`
- `usage_missing_count` - measure-phase successes with `usage_missing`
- `completion_tokens_per_second` - `null` when any measured success has
  `usage_missing` without a tokenizer count to fill the gap; otherwise a rate
- `completion_tokens_source` - `"server_usage"` or `"tokenizer_fallback"` when
  the rate is present
- `price_per_hour` / `price_provenance` - declared `$/hour` from
  `--price-per-hour` (`"cli"`) or `sut.cost.price_per_hour` (`"sut"`); both
  `null` when absent. CLI overrides SUT.
- `cost_per_million_output_tokens` - `price_per_hour / (completion_tokens_per_second * 3600) * 1e6`
  when both inputs are usable; otherwise `null` (always present in JSON)
- `observed_concurrency` - optional client outstanding-request snapshot:
  `cap`, `in_flight_mean` / `in_flight_p50` / `in_flight_max`,
  `cap_engagement_fraction` (acquires that blocked on the semaphore),
  `acquire_count`, `wait_count`
- `connect_s` / `prefill_s` / `decode_s` / `decode_tok_s` - type-7
  distributions over measured successes (empty `n=0` when absent)
- `isl_osl` - optional runtime ISL/OSL validation vs `--isl-target` /
  `--osl-target` or `--prompt-mix-report`: targets, tolerances, measured
  mean/p50, and mismatch counts

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

## Security and provenance

`environment` is client-observed (OS, architecture, optional hostname). The
`sut` block is **declared by the operator**, not measured by the client -
`provenance` is always `"declared"`. For publication runs use
`--sut <file> --require-sut` (implies `--redact-hostname`). See
[Publishing a result](../README.md#publishing-a-result).
