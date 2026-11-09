<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Historical schema fixtures

- `v171-non-reasoning.json` is the fixed-input v1.7.1 golden.
- `v171-released-*` and `v172-*` request/summary, strategic JSON/CSV and
  telemetry fixtures were captured from their respective released source
  versions against a CPU-only scripted response with one reasoning delta, one
  visible answer and reported usage. The v4 request headline is visible-answer
  TTFT; the separately recorded first-any clock is earlier. Timing values are
  only recorded fixture evidence, not performance measurements.

Machine-specific identity and paths, credentials, configuration and run IDs
were removed or replaced with neutral placeholders. One unknown optional field
was deliberately added to the telemetry fixture to exercise reader tolerance.

## vLLM logger serialization fixture

`vllm033-logger-synthetic-config.prom` is unmodified Prometheus exposition from
vLLM 0.33.0's `PrometheusStatLogger`, generated with
`export_vllm033_metrics.py`. The logger source file matches commit
`4ba787ec9959e566d3b587a796dff07673324da5`; the script checks its SHA-256 and
package versions. Configuration and scheduler values were injected, not
measured: no model engine, offloading connector or GPU was run. The output tests
real logger syntax and labels, not deployed-server behavior or physical tiers.
Process counters and timestamps vary if regenerated.

`vllm033-offload-synthetic-stats.prom` is unmodified output from the same
logger configured with a real vLLM model configuration built from the pinned
Qwen3.5-0.8B revision and `OffloadingConnector`. The reproducer
`export_vllm033_offload.py` checks the model config, tokenizer and logger source
hashes. Scheduler and connector statistics were injected; the connector and
model did not execute, and the local wheel lacked `vllm._C`. This fixture tests
serialization of APC, external and offload counters and histograms, not a live
server, transfer, physical-tier observation or GPU.
