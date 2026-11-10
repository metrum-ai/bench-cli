<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# LMCache MP source slice and certification procedure

`lmcache057-mp` selects a separate MP profile for source
`236f3eec32a2d330662a1795d90a01a1e0782c49`, stable v0.5.7. The existing legacy
`lmcache` slice remains unchanged. MP families and legacy `lmcache:*` families
have separate semantics and are never aliases.

The reviewed official linux/amd64 image is
`lmcache/vllm-openai@sha256:3be0a977f2aa4bf1acd29d5ae1e18716c6660c4733e1528caaafd758cb452cbb`,
configuration
`sha256:ef175de2e521ec563ec90af395db4e4c5f8103424a2aa2d085c868e13e1cfaa7`.
The latest stable upstream release and official registry stable tag were
checked together. The image's package layer
`sha256:13be52079df55067a612461a4960dd2d4fdf02ba2c3036563c9f619756e627e8`
was streamed and its compressed digest verified. Installed package metadata
reports LMCache 0.5.7 / `g236f3eec` and stable vLLM 0.33.0 / `g4ba787ec9`.
The MP connector, lookup/L1/engine/transfer metric subscribers and OTel
initialization file are all byte-identical to that upstream revision. The
runner independently hashes these files inside the actual container.
This is content/package evidence beyond a publisher label, not a live fit test.

## Hybrid architecture support

The source [MP connector](https://github.com/LMCache/LMCache/blob/236f3eec32a2d330662a1795d90a01a1e0782c49/lmcache/integration/vllm/lmcache_mp_connector.py)
inherits vLLM's `SupportsHMA`, validates KV-cache groups and implements
`request_finished_all_groups`. The pinned
[Qwen3.8 recipe](https://github.com/LMCache/LMCache/blob/236f3eec32a2d330662a1795d90a01a1e0782c49/docs/source/recipes/qwen3_5.rst)
uses MP with prefix caching, `--mamba-cache-mode align` and separate object
groups. Recurrent state and attention pages remain byte-opaque. The source
supports the primary model's hybrid layout; actual FP8 image/model/kernel
operation still requires the real Stage-0 measurement.

The model remains `Qwen/Qwen3.8-27B-FP8` at
`c3a462cc3d38cf351f3b26cbd763469eeb74ef81`, with text-only requests.
No alternate checkpoint is selected. The in-process connector's single-group
limitation does not apply to this selected MP connector.

## Metric semantics and exporter fixture

[Lookup metrics](https://github.com/LMCache/LMCache/blob/236f3eec32a2d330662a1795d90a01a1e0782c49/lmcache/v1/mp_observability/subscribers/metrics/lookup.py)
produce `lmcache_mp_lookup_requested_tokens_total` and
`lmcache_mp_lookup_hit_tokens_total`. They describe chunk-aligned external
lookups, exclude the engine's local prefix and sub-chunk tails, and count
contiguous-prefix hits. The numerator and denominator preserve the same model
and cache-salt labels. Whole-prompt or native-plus-external counters are not
substituted for this pair.

[L1 metrics](https://github.com/LMCache/LMCache/blob/236f3eec32a2d330662a1795d90a01a1e0782c49/lmcache/v1/mp_observability/subscribers/metrics/l1.py)
emit `lmcache_mp_l1_write_chunks_total`; its source unit is chunks (keys across
object groups), not stored tokens or resident bytes.
[Engine metrics](https://github.com/LMCache/LMCache/blob/236f3eec32a2d330662a1795d90a01a1e0782c49/lmcache/v1/mp_observability/subscribers/metrics/engine.py)
emit `lmcache_mp_num_chunks_loaded_total` for positive retrieved chunks delivered
to the engine, preserving worker identity. These are the two required positive
store/load observations, alongside the required token pair. No chunk-to-token
or byte conversion is performed.

Optional submitted/finished transfer counters retain event units. Finished
transfers include operations that committed or retrieved zero chunks, so they
cannot replace positive store/load-volume evidence. Tier labels and source
counts do not prove physical residency or physical transfer attribution.

`tests/fixtures/export_lmcache057_metrics.py` executes the full upstream metric
subscriber classes and helpers on synthetic events, through OTel SDK 1.40.0,
Prometheus exporter 0.61b0 and prometheus-client 0.25.0. Source hashes are checked.
The checked-in exposition is actual subscriber/exporter serialization, without
EventBus threading, model inference or GPU work. Rust tests preserve tokens,
chunks and events, and reject old revisions, missing families and legacy aliases.

## BC-KV-005 execution contract

The registered `KVL` adapter implements the
[shared frozen input contract](KV_ENGINE_FROZEN_INPUTS.md) and invokes
`lane-kvl.sh` for one selected case. The r4 Stage-0 adapter remains closed to KV
images. The r5 checklist requires actual image/GPU/source provenance, launcher
terminal outcome, smoke-intake rejection and proven scoped cleanup.

In addition to the ordinary frozen identity, input and tool pins, configure:

- `kv_case: BC-KV-005`, full `source_revision`, `connector_mode: mp`, distinct
  `port`, `proxy_port`, `metrics_port`, and `settle_seconds` in 1..10.
- `server_flags` including `--enable-prefix-caching`. The runner fixes
  `--mamba-cache-mode align`; disabling HMA, disabling APC, or adding a second
  connector configuration is rejected.
- `kv_connector` equal to
  `{"kv_connector":"LMCacheMPConnector","kv_role":"kv_both","kv_connector_module_path":"lmcache.integration.vllm.lmcache_mp_connector","kv_connector_extra_config":{"lmcache.mp.host":"tcp://127.0.0.1","lmcache.mp.port":5555}}`.
- `server_env: {"LMCACHE_TRACK_USAGE":"false"}` and
  `mp_server: {"chunk_size":784,"l1_size_gb":16,"separate_object_groups":true,"eviction_policy":"LRU"}`.
  Choose the L1 size from the measured host-memory fit; 16 above illustrates
  the field, not an execution measurement. The accepted bound is 1..128 GiB.
- Hash-pinned plain prompt JSONL sets `prime-seed`, `prime-reuse`, `seed`,
  `reuse`, `unique`. Priming initializes lazily emitted exporter families with
  a separate prefix and is excluded from measured phase accounting. Seed/reuse
  share a frozen prefix set; unique prompts are distinct and disjoint.
  Freeze actual tokenized lengths and ensure enough complete 784-token chunks.
- A real hash-pinned `fit` object and its underlying measurement evidence,
  including exact image/model/revision, server flags, environment, connector,
  MP server settings, GPU identity/driver, total and peak GPU memory, host
  available/required bytes, maximum prompt length and runtime compatibility.
  Both GPU and host headroom must be positive. The runner rechecks the actual
  driver and free memory immediately before starting either service.

The image and complete model snapshot are staged first. One owner-labelled
container runs both the local MP service and vLLM. The MP RPC port stays inside
that container; inference and MP metrics are published only on loopback.
Removing the owned container reaps both services. No image pull, remote model
fetch or source build occurs during execution.

Before each measured phase, the runner resets only vLLM's native prefix cache,
with a successful endpoint receipt. MP cache/counters remain in the same epoch.
Positive MP lookup hits and loaded chunks are still mandatory after this reset;
engine-local prefix reuse cannot substitute for connector evidence.
The independent proxy records exact request identities, settings, unchanged
SSE and server usage. MP and vLLM exporter snapshots each have quiet, before,
after and settled brackets. The independent vLLM request-success count must
match the traced completed population. Container and MP PID/start identities
must remain unchanged; all source series must remain monotonic with stable
labels. Missing baselines never become zero. No cross-worker or cross-salt
aggregation is permitted.

The seed window requires positive stored chunks. Reuse requires positive
requested/hit tokens with hit <= requested and positive chunks loaded into the
engine. Required profile coverage, response completion, units and provenance
must agree. Missing or incompatible evidence blocks the case and is written
as FAIL, with PENDING retained only as coordination metadata.
Passing records remain provisional until common owned-container cleanup;
the adapter additionally reconciles the ownership label and all three ports.

## Fit and scheduling inputs

Primary tensor payload is approximately 28.74 GiB, before scales, runtime
buffers, graphs, KV/recurrent state and MP host-memory reserves. This planning
lower bound does not establish GPU fit. H100 80 GB or RTX PRO 6000 Blackwell
requires an actual exact-image/model/kernel fit with positive GPU/host headroom.
The full model, tokenizer, template, recipe flags and memory allocation must be
bound to the Stage-0 result.

Plan approximately 50 minutes including startup and cleanup. This is an
estimate, not a measured runtime or the independently frozen case cap.
No GPU fit test or live certification was run while preparing this change.
