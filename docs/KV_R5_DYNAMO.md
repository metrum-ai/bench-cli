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

BC-KV-006 pins `Qwen/Qwen3-VL-32B-Instruct-FP8` at
`4bf2c2f39c37c0fede78bede4056e1f18cdf8109`, with **text-only** requests.
Record `model_selection_reason` exactly as
`DynamoConnector 1.6.2 lacks HMA; hybrid SSM unsupported`.
The connector addresses cache group 0 and does not implement `SupportsHMA`;
Qwen3.8's hybrid layout is incompatible. The full-attention checkpoint is
approximately 33 GB in FP8; weight size alone does not establish fit.

The pinned [model config](https://huggingface.co/Qwen/Qwen3-VL-32B-Instruct-FP8/blob/4bf2c2f39c37c0fede78bede4056e1f18cdf8109/config.json)
declares `Qwen3VLForConditionalGeneration`. The stable vLLM 0.31.0
[registry](https://github.com/vllm-project/vllm/blob/db9527a46873454610df6dbedf79a36d6bf1a7f6/vllm/model_executor/models/registry.py)
registers that architecture in `qwen3_vl`. This establishes source support.
The runner separately checks the exact image's installed registry, imports
the implementation, matches staged architecture and checks the reviewed
source hashes in `VLLM_SOURCE`. `model-capability.json` retains this runtime
result; offline review is not an image execution claim.

`--language-model-only` disables modality inputs in the
[0.31.0 configuration](https://github.com/vllm-project/vllm/blob/db9527a46873454610df6dbedf79a36d6bf1a7f6/vllm/config/multimodal.py).
The runner prohibits overrides and requires actual proxy evidence that every
message has string text content. Image/video/embedding content is rejected.
Single-GPU topology is explicit. Freeze `cpu_cache_gb`, server flags,
GPU/context/KV geometry and verified host RAM; the runner checks a host RAM
reserve in addition to the requested CPU cache.

Stage-0 must measure at least 25% KV headroom on the bound H100 80 GB or
RTX PRO 6000 Blackwell 96 GB. The runner retains the actual 0.31.0 worker
`Available KV cache memory` profiling output and the independently captured
GPU total, recomputes a conservative fraction allowing for reported rounding,
and fails below 25%, on ambiguous profiles, or on manual KV sizing overrides.
This is a profiled allocation budget, not physical occupancy or tier attribution.
