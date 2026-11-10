<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# vLLM 0.33.0 target-prefix eviction observer

BC-KV-002 and BC-KV-009 use `kv_r5_eviction_observer.py`, pinned to
vLLM source `4ba787ec9959e566d3b587a796dff07673324da5`. This observer requires
an unmodified engine with KV events enabled. It does not infer eviction from
an aggregate hit rate, memory utilization, a configured tier or TTFT.

## Source interpretation

In [BlockPool](https://github.com/vllm-project/vllm/blob/4ba787ec9959e566d3b587a796dff07673324da5/vllm/v1/core/block_pool.py),
`_maybe_evict_cached_block` removes the block's hash entries from the lookup
map, resets its hash and emits `BlockRemoved` with medium `GPU` and the cache
group ID. `cache_full_blocks` and `cache_partial_block` emit corresponding
`BlockStored` events. Several physical blocks can carry the same hash: the
observer replays a **multiset** of entries. One removal does not prove absence
when a second copy remains. Partial-to-full promotion removes prior aliases;
those removals also participate in the multiset.

The resulting claim is **absence of the target's GPU cache lookup entries**.
The events do not prove zeroized device bytes, physical occupancy, transfer
bytes per block or a physical tier. CPU/storage events cannot prove GPU
residency. Existing native-load-byte and TTFT conditions remain separate.

[KVCacheManager](https://github.com/vllm-project/vllm/blob/4ba787ec9959e566d3b587a796dff07673324da5/vllm/v1/core/kv_cache_manager.py)
annotates stores with the actual group's spec kind. Full-attention events bind
opaque hashes to their token-prefix chains through parent hashes and token
IDs. Mamba's align-mode mask can omit hashes while retaining a longer token
range in the event; its token array must not be zipped to the hash array.
Instead the observer binds Mamba hashes through the full-attention chain,
using the shared `Request.block_hashes` boundaries. Missing bindings, unknown
spec kinds, unsupported LoRA/multimodal/salt/lookahead hashing and conflicting
bindings fail closed. Masked empty stores carry no residency change. Partial
stores are accepted only with positive size no larger than the frozen group
size. The target must include every aligned full-attention boundary and its
aligned Mamba endpoint. Every observed partial target entry is also required
to leave the map.

Requests must use incremental KV-event reporting. The source's `full` mode
can report reused entries without allocating them, which would invalidate
multiset counts. The runner rejects that mode and unreviewed request/server
options. Frozen geometry must include every group; the observer accepts only
full-attention and Mamba groups with an aligned target endpoint.

## Complete event capture and drained windows

The [stock publisher](https://github.com/vllm-project/vllm/blob/4ba787ec9959e566d3b587a796dff07673324da5/vllm/distributed/kv_events.py)
serializes tagged msgspec structs as MessagePack batches and provides a ROUTER
replay socket. The runner enables both event sockets inside the owned
container. A bounded DEALER client runs through `docker exec` and requests
sequence zero. No new host port is published. Raw multipart frames are saved
as base64, including the empty delimiter and end marker. The host decoder
uses only the Python standard library and rejects malformed payloads,
duplicate map keys, unsupported values, gaps, truncation, wrong topic/rank,
restarts and changed historical bytes. Replay loss never becomes a pass.

Before and after every phase, the runner calls the stock
[pause API](https://github.com/vllm-project/vllm/blob/4ba787ec9959e566d3b587a796dff07673324da5/vllm/entrypoints/rl/online/api_router.py)
with `mode=wait&clear_cache=false`, verifies `/is_paused`, then captures
metrics and replay. These routes require explicit `VLLM_SERVER_DEV_MODE=1`;
the serving port remains bound to host loopback. No engine patch or collector
change is installed. The
[engine fence](https://github.com/vllm-project/vllm/blob/4ba787ec9959e566d3b587a796dff07673324da5/vllm/v1/engine/core.py)
waits until `has_work()` is false and synchronizes the device. Scheduler
liveness includes pending native connector jobs, reset fences and manager
work; thus the fence drains native transfers as well as live requests.

While paused, replay continues until two consecutive complete transcripts
are identical. The pinned publisher services replay before removing one
queued batch on each loop. Two equal replies under a completed drain fence
therefore establish an empty publisher tail. Attempts, bytes, batches, HTTP
calls and the overall case are bounded; failure to settle blocks execution.
The engine is resumed immediately before the candidate invocation. The
initial engine must have no admitted requests or cache-event batches. No
cache reset is permitted in the observed epoch.

The independent completion counter has five disjoint FinishReason bins for
one model and engine 0. Their monotonic deltas must equal the proxy-observed
frozen request count, with running/waiting gauges zero at each fence. Any
outside-bracket event or admitted request fails. The rehit bracket contains
exactly the frozen A request, with prior native work drained; native transfer
activity in this window cannot be assigned to another admitted request.
The independent rehit query delta must be positive and GPU hit delta zero.
The reported cache-configuration labels must match the frozen geometry.
Worker PID/start-time/command identities must remain unchanged.

The target's entries must be present before pressure, absent after pressure,
and still absent before rehit, in every frozen group. The evaluator derives
`kv-exclusive-window.v2` from the retained raw files. A receipt containing
only true booleans cannot supply this proof. The four native-offload
conditions and complete 7200-second startup/workload/cleanup envelope remain
unchanged.

## Frozen input contract

Set `eviction_observer` to `vllm033-kv-events.v1` in the engine specification.
Add these file pins to `inputs` so the existing frozen envelope covers them:

- `eviction_plan`: JSON `schema_version: "vllm033-eviction-plan.v1"`,
  `target_request` (the complete text chat body), `prefix_token_ids` (nonempty
  integer array), `groups` (ordered contiguous `group_idx`, `kind`,
  `block_size` objects), and `cache_config` (reported label values, including
  `block_size` and `num_gpu_blocks`).
- `kv_group_inventory`: independently captured and reviewed engine inventory
  with `source_revision`, exact `image`, `model_revision`, matching `groups`,
  and nonempty `raw_evidence` file-pin array. Raw inventory evidence is a
  transitive frozen input: also list each file pin directly in `inputs`.
  A guessed group list or logical-counter-to-byte
  conversion is insufficient. Stage-0 must establish this inventory before
  freezing execution; the observer does not manufacture it.
- `eviction_observer_module`: a pin of `kv_r5_eviction_observer.py` itself.

`target_prefix_sha256` is SHA256 of the compact JSON integer array
`prefix_token_ids`. The actual `/tokenize` response must begin with those IDs
and contain at least one additional prompt token, because the cache lookup
requires recomputation of the last prompt token. Tokenization uses the same
messages and template kwargs as the frozen rehit wire body. Its raw response
is retained. Freeze explicit plain text pressure requests that exclude A;
shared target-prefix entries that remain or reappear prevent the eviction gate
from passing. Each phase has zero warmup requests.

The `source_files` map must include the runner's existing slices and every
path/hash in the observer's `SOURCE_FILES`, under the installed vLLM import
root. The runtime checks the actual bytes in the exact owned image. The
immutable image also binds its msgspec and ZMQ implementation; no host decoder
dependency is installed at execution time.

## Offline validation

`tests/fixtures/vllm033-kv-events-synthetic.json` contains injected synthetic
full-attention/Mamba stores, duplicate entries and removals. Its bytes were
produced by the unmodified upstream EventBatch/KVCacheEvent structs and the
actual `publish`, `_publisher_thread` and `_service_replay` methods, using
msgspec 0.19.0 and fake sockets/queues. This validates serialization and
interpretation, not GPU behavior. Reproduce with:

```sh
python scripts/validation/cert/generate_kv_event_fixture.py kv_events.py fixture.json
python -m unittest discover -s scripts/validation/cert -p 'test_kv_r5*.py'
```

The generator requires the pinned source file's exact SHA256 and msgspec.
Offline tests cover duplicate copies, malformed streams, missing groups,
conflicting token bindings, counter resets, competing requests, epoch changes
and an unsettled replay tail. Real Stage-0, target eviction and native-transfer
evidence remain necessary before either case can pass.
