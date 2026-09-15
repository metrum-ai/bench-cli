<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Comparison with other inference benchmarks

Metrum AI Bench is designed for one client process measuring four
OpenAI-compatible modalities and distributing requests over multiple
endpoints. It is not an MLPerf compliance harness.

- NVIDIA GenAI-Perf has broader telemetry and reporting. Comparable controls
  are `--request-rate`, Poisson arrival, tokenizer counts, and SLO goodput.
- vLLM `benchmark_serving.py` has broad dataset integrations. Comparable
  controls are `--request-rate`, `--max-concurrency`, `--seed`,
  `--ignore-eos`, and local tokenizer counts.
- GuideLLM includes sweep and HTML reporting. Metrum AI Bench does not yet
  claim either strategic Wave (iii) feature.
- MLPerf LoadGen specifies audited workload and accuracy rules. Metrum AI
  Bench reports open-loop scheduled latency and reproducibility metadata but
  its output is not MLPerf-compatible.

Metric comparisons are valid only when tokenizer, prompt sequence, sampling
parameters, warmup, window, endpoint topology, and SLO definitions match.
Unlike clients that target a single endpoint, pooled multi-endpoint results
are explicitly marked `pooled_mixture`; use `per_endpoint` for diagnosis.
