<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# JSONL output schema

Each line is a complete JSON object and carries `schema_version`.

## Request v2

`metrum-ai-bench.request.v2` records are flushed immediately on completion:

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

## Summary v2

`metrum-ai-bench.summary.v2` contains measured attempted/success/error counts,
rates, type-7 distributions, coordinated-omission-corrected latency,
throughput-bin dispersion, SLO goodput, `pooled_mixture`, full
`per_endpoint` distributions, environment metadata, and `partial`.

Warmup request lines remain in the file for audit but are excluded from
summary distributions. Ctrl-C stops issuance, drains started requests, and
writes a partial summary. A hard kill may leave valid request lines without a
summary; consumers must accept that recoverable prefix.

Legacy summary objects are temporarily appended by the modality binaries for
existing consumers. New consumers must select records by `schema_version`,
not line position.
