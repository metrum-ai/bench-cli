<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Stable Dynamo KVBM execution

BC-KV-006 uses official Dynamo 1.6.2's vLLM runtime and the
`dynamo162-kvbm` source profile, revision
`28f9765d98ded77149a94d0defee6d54e97f6741`. The older `dynamo-kvbm` profile
remains pinned independently; neither accepts the other's source declaration.
There is no inferred image-label alias.

The [1.6.2 metric implementation](https://github.com/ai-dynamo/dynamo/blob/28f9765d98ded77149a94d0defee6d54e97f6741/lib/llm/src/block_manager/metrics_kvbm.rs)
has SHA256 `cce41a4476750722974b12252e4edbe269f9c87c33f85666bd35751f528e25ee`
and is byte-identical to the older profile's reviewed file. Its registry adds
the `kvbm_` prefix, uses integer counters for blocks/tokens and sliding-window
ratio gauges. Object failure counters count blocks. The
[name definitions](https://github.com/ai-dynamo/dynamo/blob/28f9765d98ded77149a94d0defee6d54e97f6741/lib/runtime/src/metrics/prometheus_names.rs)
and registration were reviewed together. No blocks-to-bytes conversion or
physical-tier inference follows from those declarations.

The fixture `dynamo162-registry-synthetic.prom` was captured from Prometheus
0.14.0's text encoder using the upstream registration block and registry
implementation, with injected counters 8/4/64 and rates 0.5/0.25/0. The network
endpoint/thread was omitted; the valid-ASCII metric-name sanitizer was an
identity stand-in. This is actual registry serialization with synthetic
observations, not a running KVBM engine or GPU capture.

Use `kv_r5_dynamo_runner.py` with the [execution contract](KV_R5_EXECUTION.md).
It starts a single owned vLLM process from the official runtime with
`DynamoConnector`, `kv_both` and module
`kvbm.vllm_integration.connector`. This exercises the official documented
single-box connector integration; it does not need a distributed frontend.
Metrics are enabled explicitly on container port 6880. Freeze phases
`populate`, `reuse`, `control` with repeated-prefix and distinct control wire
identities. Every required family must parse, counters must remain same-series
and monotonic, and matched-token delta must be positive in the reuse phase.
Tier attribution remains Unknown. Raw before/after pages and request responses
are retained; optional failure counters do not become a substitute reuse metric.

The [runtime Dockerfile](https://github.com/ai-dynamo/dynamo/blob/28f9765d98ded77149a94d0defee6d54e97f6741/container/templates/vllm_runtime.Dockerfile)
installs the KVBM wheel only when `ENABLE_KVBM` is true. The published
image's manifest/config is insufficient proof of installation. Before serving,
the runner checks the actual package, extension files, connector import and
reviewed connector source hashes. `inputs.kvbm_build_provenance` must pin a JSON
receipt with `image`, `source_revision`, actual `extension_sha256` path/hash
map, and `independent_build_evidence` file-pin array. Verify that evidence links
the compiled extension to the reviewed source; an operator-created label or
receipt assertion alone is insufficient. No wheel is installed as a fallback.
Missing imports, unbound extension provenance or non-operational KVBM block
execution. Availability of the published wheel remains unverified offline.

Prefer Qwen3.8-27B-FP8 revision c3a462cc with the complete model/tokenizer lock;
the vLLM 0.31.0 base still requires an architecture/kernel and single-GPU fit
check. Otherwise record the newest supported official Qwen and its rationale.
Qwen3-8B requires an explicit last-resort notice. Freeze `cpu_cache_gb`, server
flags, GPU/context/KV geometry and verified host RAM; the runner checks a host
RAM reserve in addition to the requested CPU cache. H100 80 GB or RTX PRO 6000
Blackwell requires actual model and KV fit, not just inventory capacity.
