<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Standalone TRT-LLM 1.2.1 exporter scope

BC-KV-007 uses stable standalone TensorRT-LLM 1.2.1 and source
`376f7e1bd8ed543f75014309e3fd4b237e9b0e73`. Select `trt-llm121` explicitly.
This is a separate profile from the newer `trt-llm` source slice; no release
alias, Triton metric alias or cached-token substitute is introduced.

The [stable collector](https://github.com/NVIDIA/TensorRT-LLM/blob/376f7e1bd8ed543f75014309e3fd4b237e9b0e73/tensorrt_llm/metrics/collector.py)
defines one request-completion counter and four duration histograms:

| Family | Type / units |
| --- | --- |
| `trtllm_request_success_total` | Cumulative completed requests, with source finish-reason labels. Not cache queries. |
| `trtllm_e2e_request_latency_seconds` | End-to-end request duration histogram. |
| `trtllm_time_to_first_token_seconds` | First-token duration histogram. |
| `trtllm_time_per_output_token_seconds` | Output-token duration histogram. |
| `trtllm_request_queue_time_seconds` | Queue duration histogram. |

Histogram buckets/counts retain observation units and sums retain seconds.
The generated `_created` gauges are implementation timestamps, not cache
evidence, and are deliberately outside the narrow profile. Counter label groups
and histogram boundaries remain distinct. No histogram quantiles are reconstructed.

The [OpenAI server](https://github.com/NVIDIA/TensorRT-LLM/blob/376f7e1bd8ed543f75014309e3fd4b237e9b0e73/tensorrt_llm/serve/openai_server.py)
mounts this collector at `/prometheus/metrics` only when `return_perf_metrics`
is enabled. `/metrics` is a different iteration-statistics endpoint and is not
accepted as Prometheus exposition. The initial collector labels use model name
`undefined`; these source labels are retained, not rewritten into model identity.
Image/model/source checks are independent of exporter labels.

**TRT-LLM cache-reuse attribution is unavailable on 1.2.1.** Cached tokens are
explicitly `{value: null, status: "unknown", reason: "not exported by 1.2.1"}`.
This condition is expected for this stable source and is recorded in the result;
it never becomes zero or a physical-tier claim. PASS requires all required
source-defined families, complete source-defined bucket layouts, stable series,
monotonic consistent deltas, and request-completion/histogram-count agreement
with independently recorded successful responses from the actual server.

`trtllm121-collector-synthetic.prom` is a capture of the unmodified collector
loaded with its real enum module and `prometheus-client==0.23.1`, with injected
synthetic completed-request metrics (E2E 0.5s, TTFT 0.1s, TPOT 0.02s, queue
0.05s). No serving engine, model or GPU ran. The collector SHA256 is
`68a904084f36e491d34f035efa0c8bded13330f59d0eb28821e0eb779d59c475` and enum
SHA256 is `f762c5c9f5c37ef78b697af1deb63b3809901f3245a273b42ac3580102b1873e`.
These exporter fixtures establish serialization compatibility, not live evidence.

Use `kv_r5_trtllm_runner.py` with the [frozen-input contract](KV_R5_EXECUTION.md).
Freeze `inputs.engine_config` as a JSON configuration (also valid YAML), enabling
`return_perf_metrics: true`, single-GPU parallelism, bounded batch/context/token
limits and `kv_cache_config.free_gpu_memory_fraction`. Freeze `primer` then
`measure` phases: the primer is explicitly held out and initializes lazy metric
labels. Every measured invocation has zero product warmup and uses the exact
frozen streaming workload. This narrow gate accepts one finish-reason series;
new label groups or partial timing coverage fail closed. No unknown histogram
family is replaced by zero.

The model is official
`Qwen/Qwen3-30B-A3B-Instruct-2507-FP8@5a5a776300a41aaa681dd7ff0106608ef2bc90db`.
Its `Qwen3MoeForCausalLM` architecture appears in the
[1.2.1 model list](https://github.com/NVIDIA/TensorRT-LLM/blob/376f7e1bd8ed543f75014309e3fd4b237e9b0e73/docs/source/models/supported-models.md).
Qwen3.8's `Qwen3_5ForConditionalGeneration` hybrid architecture does not appear
there. Qwen3Next's larger official FP8 checkpoint leaves insufficient single-GPU
KV headroom, so the smaller official Qwen3MoE FP8 checkpoint is used. Published
tensor counts imply about 31.2 GB of weight storage; this is a planning figure,
not measured device use. The complete weight/tokenizer lock and architecture,
kernel and dtype compatibility remain runtime prerequisites.

The local snapshot mounts at `/model`, whose basename is the source's served
model ID (`model`). The manifest separately retains the actual upstream model
and revision. Startup memory profiling must show **at least 25% KV headroom** on
the bound H100 80 GB or RTX PRO 6000 Blackwell. The runner retains and parses the
[source profiling report](https://github.com/NVIDIA/TensorRT-LLM/blob/376f7e1bd8ed543f75014309e3fd4b237e9b0e73/tensorrt_llm/_torch/pyexecutor/_util.py),
bounds its two-decimal rounding conservatively, checks total memory against GPU
inventory and rejects missing, ambiguous or insufficient profiles. This reports
available KV budget after runtime peak-memory profiling, not physical occupancy
or proof that a cache tier participated. Startup/primer and cleanup remain inside
the case time envelope. Live image/model/kernel and memory checks are outstanding.
