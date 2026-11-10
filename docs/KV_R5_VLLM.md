<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# vLLM KV case execution

Use `scripts/validation/cert/kv_r5_vllm_runner.py` with the
[frozen-input contract](KV_R5_EXECUTION.md). Image v0.33.0, its amd64 child,
source revision 4ba787ec and Qwen3.8-27B-FP8 revision c3a462cc are explicit
constants. The runtime checks the source bytes at the actual import location.
On H100 freeze `--max-num-seqs 256` in `server_flags` and enable APC explicitly.
Freeze context length, GPU cache budget, model file lock and measured fit before
running. Readiness and memory capacity alone do not establish model correctness.

| Case | Ordered phases and additional requirements |
| --- | --- |
| BC-KV-001 | `cold`, `warm`, `unique`; at least 16 paired shared-prefix requests and 16 distinct unique-prefix controls with comparable generation settings. Warm requests repeat the frozen cold message hashes. No reset between cohorts. Positive cached-token usage and hit/query deltas must agree with independently captured source values and product phase output. |
| BC-KV-002 | `cold`, `populate`, `pressure`, `rehit`; native CPU KV offload flags, verified host RAM and an independent target-prefix observer are required. Set `native_cpu_load: true` on the rehit phase. Positive completed CPU-load bytes, exactly zero GPU prefix-hit tokens, strictly lower rehit TTFT and independent GPU eviction evidence must all hold in an exclusive settled window. The entire case including startup/cleanup is capped at 7200 seconds. |
| BC-KV-008 | Named session stages; each has `reset_prefix_cache: true`, at least four deterministic measured sessions of at least three turns and separate held-out warmup sessions. Freeze closed-loop mode and source turn numbering. Exact wire order, session/turn mapping, cold markers and measured/warmup boundaries are checked. Each stage retains a successful reset acknowledgement and raw counter pages proving zero prefix-hit delta on its first measured request. Dev mode is explicitly enabled for 008. Outer counter brackets include warmup and are labeled accordingly; they are not compared with measured-only product deltas. |
| BC-KV-009 | `cold`, `populate`, `warm`, `pressure`, `rehit`, `control`; freeze tokenized target prefix, KV geometry and pressure sequence. Warm reuse, independently observed target eviction and rehit/miss accounting are required. |

For non-session cases freeze zero product warmup requests: independent brackets
must match the measured-only product phase, including start/end values. Enable
streaming usage, source-matched `vllm4ba-apc` telemetry, native phase capture and
required phase coverage. Each invocation has one measured concurrency-one stage.
The runner records per-cohort count, median and type-7 p95 without a speedup gate.
Cache cohort labels derive from returned cached tokens, never turn number.

Offload and pressure use the source-bound
[KV-events observer](KV_R5_EVICTION_OBSERVER.md). Freeze
`eviction_observer: "vllm033-kv-events.v1"`, `target_prefix_sha256`,
`inputs.eviction_plan` and `inputs.kv_group_inventory`. Arbitrary executable
receipts and aggregate hit rates cannot supply eviction evidence. The runner
retains raw replay frames, independent counter pages, pause/resume replies,
server tokenization and process epochs; evaluation reconstructs the proof.
A missing observer input, incomplete stream, unknown group geometry or failure
to observe complete target eviction blocks both cases. Offline tests do not
turn these gates into a live certification result.

The existing exporter fixture tests instantiate real logger serialization with
synthetic statistics. They are not a deployed-model capture. This runner saves
raw live pages when executed; a reviewed live APC capture is still required
before closing the captured-exporter coverage gap. No GPU case is certified by
the offline tests in this change.
