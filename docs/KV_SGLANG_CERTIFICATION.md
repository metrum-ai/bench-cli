<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# SGLang source slice and live certification procedure

`sglang0523` selects source `297e54fd105e974cdf401e5b4d20253a22c16f90`
(v0.5.23). It does not change `sglang`, accept abbreviated revisions or create
a release alias. The reviewed official image is
`lmsysorg/sglang@sha256:f832b5c39559c9caa900b22ad37c76fcce66470676855c42052102c537812f1f`;
linux/amd64 manifest
`sha256:f7876bef696ccf9cf75a2bf7ccb82970f5fac577f082fb8ca95c9a59d2160eb5`,
configuration
`sha256:f3a303d4028dc4597e312c96ada28fa19509f46559601a94e89dc630ac1ef152`.
The latest stable upstream release and the official registry's stable tag were
checked together. No stable official image was established for the older
`188cf7da` slice; this is a separate revision-specific profile.

The image's source-install layer
`sha256:f50a2af9072ad99dc16b692684457cd93bf4adc4b99b1113e3fd5b8af39d99cc`
was read and its compressed digest verified. Its source checkout Git HEAD is
`297e54fd105e974cdf401e5b4d20253a22c16f90`; the installed checkout's
`python/sglang/srt/observability/metrics_collector.py` matches upstream SHA-256
`745d697dcf71ab8aa148059bccba27a286658bd736ee4b8a08489d90797f19f2`.
This checks source content beyond a publisher label. It does not establish a
running engine's compatibility; the runner checks that file in the actual container.

[Metric source](https://github.com/sgl-project/sglang/blob/297e54fd105e974cdf401e5b4d20253a22c16f90/python/sglang/srt/observability/metrics_collector.py):
`TokenizerMetricsCollector.observe_one_finished_request` increments cached
prompt tokens with `cache_source=device/host/storage`, or `total` only when
detailed attribution is absent. These views must not be summed. Zero-valued
tiers need not appear. `SchedulerMetricsCollector` sets `cache_hit_rate` as a
gauge, a ratio in [0,1]. It is neither a cumulative counter nor occupancy.
No occupancy or physical-tier observation is supplied by this profile.

`tests/fixtures/export_sglang0523_metrics.py` executes the exact selected
upstream constructor assignments, cached-token reporting block and gauge
method with synthetic inputs and prometheus-client 0.25.0. The checked-in text
is actual serialization of these source statements; it is not a full logger
instance, deployed exporter page, inference run or GPU certification.
The Rust fixture tests cover labels, units, old/new revision mismatch, missing
required cached-token coverage and invalid rates. The existing #420 regression
`cache_hit_rate_is_not_kv_occupancy` remains applicable.

## Executing BC-KV-003 or BC-KV-004

Use `case_launcher.py` and a separately frozen execution manifest, with one
selected case and `lane-kvs.sh` as launcher. The lane name is `KVS` in the lane
manifest. The existing tool pins, harness closure, actual-host binding,
watchdog limits and cleanup reconciliation remain mandatory. This procedure
adds no catalog rows or release passes by itself.

Alongside the ordinary lane fields, freeze:

- `source_revision` as the full source revision above, `kv_case` as one of the
  two case IDs, a distinct `proxy_port`, `settle_seconds` in 1..10, `owner` and
  `evidence_link`.
- `server_flags` including `--enable-metrics`; additionally
  `--enable-hierarchical-cache` for BC-KV-004 only. Freeze supported HiCache
  memory/layout/write policy flags from that exact source. Identity and cache
  disabling overrides are rejected. There is no implicit flag substitution.
- Hash-pinned `prompt_sets.seed`, `.repeat`, `.unique`, with plain
  `{"prompt":"..."}` JSONL rows. Seed and repeat each contain at least two
  requests and the same prompt set; unique has distinct prompts disjoint from
  that set. HiCache also requires `.pressure` and `.rehit`; rehit uses the
  original prefix. Freeze the actual tokenized input lengths and prefix shapes.
- Hash-pinned `fit` JSON and its hash-pinned `evidence` receipt. The fit binds
  image, model, revision, exact server flags, actual `gpu_uuid`, `gpu_class`,
  measured `gpu_memory_bytes`, `peak_gpu_memory_bytes`, `host_available_bytes`,
  `host_required_bytes`, `max_prompt_tokens`, and `runtime_compatible: true`.
  Both memory headrooms must be positive. HiCache additionally records measured
  `kv_capacity_tokens`; the pressure cohort must exceed it in server-reported
  prompt tokens. Such pressure does not itself prove physical eviction.

The engine loads only a staged offline model cache, starts in a fresh labelled
container and serves through the loopback interface. No image build, model
substitution or download happens in this procedure. The actual GPU identity
and capacity must match the fit receipt. Every cohort invokes the candidate
binary and retains prompt hashes, original/forwarded SSE and CLI output.
Repeated requests require positive server cached-token usage and a positive
same-series exporter delta. Counter/worker resets, missing families, absent
source evidence, ambiguous workers, incomplete responses or extra exporter
requests block the case. The source token counter is not interpreted as physical
residency. Quiet and settled snapshots surround every measured cohort, and the
container's image, start time, owner and restart count must remain unchanged.
New tier series have Unknown deltas; no missing baseline is replaced by zero.

An unavailable HiCache configuration or unsupported runtime remains PENDING
for coordination. The intake record is FAIL, since the existing validator does
not accept PENDING as a passing result. Records stay provisional until the
common lifecycle verifies owned-container deletion. No validation semantics
or existing case criteria are relaxed.

## Fit and scheduling inputs

The proposed checkpoint is `Qwen/Qwen3.8-27B-FP8` at
`c3a462cc3d38cf351f3b26cbd763469eeb74ef81`. Its official metadata declares
`Qwen3_5ForConditionalGeneration`, implemented in this SGLang source. Metadata
reports 24,699,207,680 FP8 and 3,082,220,272 BF16 parameters: approximately
28.75 GiB of tensor payload before scales, buffers, graphs, runtime allocations,
KV and host-cache reserves. This is a planning lower bound, not a fit result.
H100 80 GB or RTX PRO 6000 Blackwell may be selected only after the actual
image/model/kernel and headroom checks. HiCache's hybrid-state behavior and
host-memory requirements must be demonstrated on that configuration.

Plan approximately 35 minutes for BC-KV-003 and 40 for BC-KV-004, including
startup and cleanup; these are estimates, not caps or measured runtimes.
Actual GPU/host headroom and operational compatibility remain unmeasured here.
No live certification was performed while preparing these offline changes.
